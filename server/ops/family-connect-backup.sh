#!/usr/bin/env bash
#
# family-connect nightly backup: the PostgreSQL database and the attachment
# bytes, which since [storage] moved blobs out of the database are two halves
# of one backup. Neither alone restores a family's history.
#
# Install: see docs/operations.md. Run by family-connect-backup.timer.
#
# ORDER IS LOAD-BEARING: the database is dumped FIRST and the attachments are
# copied SECOND. A backup is not an instant — things happen between its two
# halves — and only one order is safe:
#
#   db then files : a photo uploaded in between has its bytes in the backup
#                   and no row pointing at them. The unclaimed sweeper's job.
#                   Harmless.
#   files then db : the same photo has a ROW in the dump and no bytes behind
#                   it. A restore that silently loses pictures.
#
# The second is the one people write by accident, because copying the big
# slow thing first feels natural.

set -euo pipefail

CONFIG="${FC_BACKUP_CONFIG:-/etc/family-connect/backup.env}"
# shellcheck source=/dev/null
[ -r "$CONFIG" ] && . "$CONFIG"

: "${PGDATABASE:=family_connect}"
: "${PGUSER:=family-connect}"
: "${ATTACHMENTS_DIR:=/var/lib/family-connect/attachments}"
: "${STAGING_DIR:=/var/backups/family-connect}"
: "${KEEP_LOCAL_DAYS:=7}"
# Where the off-box copy goes. Exactly one of these must be set — a backup
# that never leaves the machine is not a backup, it is a second copy on the
# disk that is going to fail.
: "${RESTIC_REPOSITORY:=}"
: "${RSYNC_TARGET:=}"

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }
die() { log "FAILED: $*"; exit 1; }

if [ -z "$RESTIC_REPOSITORY" ] && [ -z "$RSYNC_TARGET" ]; then
    die "set RESTIC_REPOSITORY or RSYNC_TARGET in $CONFIG — an on-box backup is not a backup"
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$STAGING_DIR"
chmod 0700 "$STAGING_DIR"
dump="$STAGING_DIR/db-$stamp.dump"

# --- 1. the database, first (see the note at the top) ---------------------
#
# -Fc (custom format) rather than plain SQL: compressed, and pg_restore can
# read a single table out of it, which is what makes a partial recovery
# possible at 3am.
log "dumping $PGDATABASE"
pg_dump -Fc --file="$dump" "$PGDATABASE" || die "pg_dump"

# A dump nobody has read back is a hope. This costs a second and catches a
# truncated or half-written file NOW, rather than on the day it is needed.
pg_restore --list "$dump" >/dev/null || die "the dump just written is not readable by pg_restore"
size=$(wc -c <"$dump")
[ "$size" -gt 1024 ] || die "the dump is $size bytes — that is not a database"
log "dump ok: $dump ($size bytes)"

# --- 2. the attachment bytes, second --------------------------------------
if [ -n "$RESTIC_REPOSITORY" ]; then
    # restic snapshots are immutable, so a file deleted on the server between
    # the dump and now still exists in every snapshot that predates the
    # deletion — which is what keeps an older dump's rows resolvable.
    # Retention is `restic forget`, run below, NOT the absence of a copy.
    log "restic: backing up $ATTACHMENTS_DIR and $dump"
    restic backup --tag family-connect "$ATTACHMENTS_DIR" "$dump" || die "restic backup"
    restic forget --tag family-connect \
        --keep-daily 7 --keep-weekly 5 --keep-monthly 12 --prune \
        || log "WARNING: restic forget/prune failed; snapshots still written"
else
    # No --delete: a blob removed between the dump and this copy would
    # otherwise vanish from the backup while the dump still references it.
    # The cost is that the mirror accumulates orphaned blobs; prune it on a
    # schedule that lags your oldest dump, never in this script.
    log "rsync: $ATTACHMENTS_DIR -> $RSYNC_TARGET"
    rsync -a --partial "$ATTACHMENTS_DIR/" "$RSYNC_TARGET/attachments/" || die "rsync attachments"
    rsync -a "$dump" "$RSYNC_TARGET/db/" || die "rsync dump"
fi

# --- 3. local staging retention -------------------------------------------
find "$STAGING_DIR" -name 'db-*.dump' -type f -mtime +"$KEEP_LOCAL_DAYS" -delete

log "backup complete"
