# Running the server

What has to be true of the box, beyond installing the binary. `README` covers
the install; this covers keeping it up and getting it back.

Everything here is about **one small server that a family depends on**. That
shapes the advice: there is no second machine to fail over to, the operator is
one person who is not on call, and the failure that matters is not downtime
but losing five years of a family's photographs.

---

## 1. Rate limiting

Shipped in `server/nginx/family-connect.conf`. Install that file, do not write
your own — the reasoning is in its comments, and one detail is easy to get
wrong in a way nothing reports:

> An **exact-match `location =` block inherits nothing** from the
> `location /api/v1/` beside it. A login block without its own `proxy_pass`
> and `proxy_set_header` lines passes `nginx -t` and then serves 404 from the
> default root. Repeat them in every block.

Two zones, keyed on the client address:

| Zone | Rate | Applies to | Why |
|---|---|---|---|
| `fc_auth` | 6r/m, burst 3 | `POST /auth/login`, `POST /auth/register` | Both hash a password with argon2id, and **login hashes even for a username that does not exist** — deliberately, so the unknown-user and wrong-password paths cannot be told apart by timing. That makes them the most expensive thing a stranger can ask for. |
| `fc_api` | 20r/s, burst 60 | everything else under `/api/v1/` | A runaway guard. Opening a chat fires a burst of reads and a family syncing after a flight fires a bigger one, so this is deliberately generous. |

Plus `limit_conn fc_conn 24` — several devices per person, counted per
address, because a household behind one NAT is a single address.

Measured behaviour, not intent: four login attempts land immediately and the
fifth is `429`, with one more allowed every ten seconds. Somebody who has
forgotten their password notices nothing; somebody working through a word list
gets six an hour. **This is why the protocol has no account lockout** — a
lockout would let a stranger deny a real member their own account.

> **If nginx sits behind another proxy** (Cloudflare, a load balancer), set
> `real_ip_header` and `set_real_ip_from` as well. Otherwise every request
> shares one zone key and the first person to sign in rate-limits everyone.

---

## 2. Memory

`Argon2::default()` allocates **19 MiB per hash**, and a bare `#[tokio::main]`
gives `spawn_blocking` a 512-thread pool. Unbounded, a login flood can ask for
~9.7 GiB and the kernel's OOM killer then picks a victim — usually
PostgreSQL, because it is the biggest thing on the box. Losing the database to
protect the chat server is exactly backwards.

Two bounds, both shipped:

- `limits.max_password_hashes_in_flight` (default **8**, ~152 MiB) — the
  server's own semaphore. Requests over it **wait**; refusing a legitimate
  sign-in during a burst would be a worse answer than a short queue. Arrival
  *rate* is nginx's job, above; peak *memory* is this.
- `MemoryHigh=512M` / `MemoryMax=768M` in the unit — the cgroup wall. If a
  bound is ever raised carelessly, systemd kills **this** service and
  `Restart=on-failure` brings it back, instead of the kernel killing
  PostgreSQL.

Raise both together on a larger box, and keep the ratio: the cgroup ceiling
must comfortably exceed `19 MiB × max_password_hashes_in_flight`.

---

## 3. Disk

Two independent protections. Use both.

**The floor (shipped, on by default).** `limits.min_free_disk_bytes` (2 GiB)
is free space uploads may not eat into. Over it, an upload is refused with
`storage_full` (507) — a refusal that says *the server* is out of room, so
clients tell the sender to try again rather than telling them their photo was
too big. The check runs before the body is read.

It is a floor for **PostgreSQL**, not a budget for photos. The database
usually shares this filesystem and handles a full disk far worse than a
refused upload does.

**A separate filesystem (yours to do, and stronger).** The floor is a
guess about how much room PostgreSQL needs. Its own volume is a guarantee:

```sh
# LVM example — sizes to taste.
sudo lvcreate -L 100G -n fc-attachments vg0
sudo mkfs.ext4 /dev/vg0/fc-attachments
sudo systemctl stop family-connect
sudo mv /var/lib/family-connect/attachments /var/lib/family-connect/attachments.old
sudo mkdir -p /var/lib/family-connect/attachments
echo '/dev/vg0/fc-attachments /var/lib/family-connect/attachments ext4 defaults 0 2' \
  | sudo tee -a /etc/fstab
sudo mount -a
sudo rsync -a /var/lib/family-connect/attachments.old/ /var/lib/family-connect/attachments/
sudo chown -R family-connect:family-connect /var/lib/family-connect/attachments
sudo systemctl start family-connect
# Only after checking a photo still opens in the app:
sudo rm -rf /var/lib/family-connect/attachments.old
```

Then a fill costs refused uploads and nothing else. Note the server's
`ensure_root()` runs at **boot**, so a volume that failed to mount is a
startup failure rather than a surprise on the family's next photo — but a
volume that unmounts while running is not detected, which is why the fstab
entry matters more than the manual mount.

---

## 4. Backups

**A database dump is not a backup.** Attachment bytes live on disk, not in
PostgreSQL, so a `pg_dump` alone restores a family's history with every
photograph missing. Both halves, or neither.

Shipped in `server/ops/`:

| File | Install to |
|---|---|
| `family-connect-backup.sh` | `/usr/local/bin/family-connect-backup.sh` (0755) |
| `family-connect-backup.service` | `/etc/systemd/system/` |
| `family-connect-backup.timer` | `/etc/systemd/system/` |

```sh
sudo install -m 0755 server/ops/family-connect-backup.sh /usr/local/bin/
sudo install -m 0644 server/ops/family-connect-backup.{service,timer} /etc/systemd/system/
sudo install -d -o family-connect -g family-connect -m 0700 /var/backups/family-connect
sudo install -d -m 0750 /etc/family-connect
sudo tee /etc/family-connect/backup.env >/dev/null <<'EOF'
# Exactly one destination. Off the box — a copy on the same disk is not a
# backup, it is a second thing that dies with the first.
RESTIC_REPOSITORY=sftp:backup@elsewhere.example:/srv/family-connect
RESTIC_PASSWORD_FILE=/etc/family-connect/restic-password
# or: RSYNC_TARGET=/mnt/offbox/family-connect
EOF
sudo chmod 0640 /etc/family-connect/backup.env
sudo chown root:family-connect /etc/family-connect/backup.env
sudo systemctl enable --now family-connect-backup.timer
sudo systemctl start family-connect-backup.service   # run one now
journalctl -u family-connect-backup.service -n 40
```

### The order is load-bearing

The script dumps the **database first** and copies the **attachments second**,
and that is not arbitrary. A backup is not an instant; things happen between
its halves:

- **database, then files** — a photo uploaded in between has bytes in the
  backup and no row pointing at them. The unclaimed sweeper's job. Harmless.
- **files, then database** — the same photo has a **row** in the dump and no
  bytes behind it. A restore that silently loses pictures.

The second is what people write by accident, because copying the big slow
thing first feels natural. Do not reorder it.

For the same reason the rsync path deliberately omits `--delete`: a blob
removed between the dump and the copy would otherwise vanish from the backup
while the dump still references it. Prune the mirror on a schedule that lags
your oldest dump. With restic this is automatic — snapshots are immutable and
`restic forget` handles retention.

The script also runs `pg_restore --list` over each dump it writes. A dump
nobody has read back is a hope, and a truncated one should be discovered on
the night it happens rather than on the day it is needed.

---

## 5. Restoring

**This procedure has been run end to end** (PostgreSQL 16, rsync
destination): 500 message rows and a sharded attachment tree wiped and
recovered, with the restored tree verified byte-identical to the backup by
checksum. Two things it turned up, both of which will bite you at 3am:

> **The two halves restore as two different users.** The database is
> PostgreSQL's, the attachment tree is `family-connect`'s. Restoring the files
> as `postgres` fails with `Permission denied` on the state directory. This is
> the step the first run got wrong.

> **`--no-owner --no-privileges`** on `pg_restore`, unless you are restoring
> onto a box with the identical role names.

```sh
sudo systemctl stop family-connect

# --- half 1: the database, as postgres ---
DUMP=$(ls -t /mnt/offbox/family-connect/db/db-*.dump | head -1)   # or: restic restore
sudo -u postgres dropdb --if-exists family_connect
sudo -u postgres createdb -O family-connect family_connect
sudo -u postgres pg_restore --dbname=family_connect --no-owner --no-privileges "$DUMP"

# --- half 2: the bytes, as family-connect ---
sudo install -d -o family-connect -g family-connect /var/lib/family-connect/attachments
sudo -u family-connect rsync -a \
  /mnt/offbox/family-connect/attachments/ /var/lib/family-connect/attachments/

sudo systemctl start family-connect
```

Verify before you believe it:

```sh
# The tree matches the backup.
diff <(cd /mnt/offbox/family-connect/attachments && find . -type f -exec md5sum {} \; | sort) \
     <(cd /var/lib/family-connect/attachments   && find . -type f -exec md5sum {} \; | sort) \
  && echo "attachments identical"

# Rows and bytes agree. Files are sharded on the key's last four characters
# REVERSED and split 2+2 — key "7-abcd" lives at "dc/ba/7-abcd", not
# "ab/cd/". Getting this backwards makes a good restore look like a failed
# one.
sudo -u postgres psql -tAq -d family_connect -c \
  "SELECT count(*) FROM messages;"
```

Then open the app and look at a photo. That is the only check that covers
everything.

**Do this once now, on a scratch box, before you need it.** A backup you have
never restored is a belief, not a plan.

---

## 6. Monitoring

`GET /api/v1/healthz` does a real `SELECT 1`, so it fails when the database is
gone rather than only when the process is. Nothing polls it — that is
deliberate: a monitor that runs on the box it monitors reports nothing about
the outage that matters.

Point an **external** uptime service at it and send the alert somewhere you
will actually look. Minimums worth setting:

| Check | Interval | Alert when |
|---|---|---|
| `GET https://<host>/api/v1/healthz` | 5 min | two consecutive failures |
| TLS certificate expiry | daily | under 14 days |
| Disk free on the attachments filesystem | hourly | under `min_free_disk_bytes` × 2 |
| `family-connect-backup.service` result | daily | last run failed or is over 48h old |

The last one is the one people skip, and it is the one that hurts. A backup
that has been failing silently for six weeks is worse than no backup, because
you have been relying on it. If your monitor cannot see systemd, this puts the
answer in a file a check can fetch:

```sh
# /etc/systemd/system/family-connect-backup.service.d/report.conf
[Service]
ExecStopPost=/bin/sh -c 'echo "$(date -u +%%FT%%TZ) $EXIT_STATUS" > /var/lib/family-connect/last-backup'
```

The App Store review notes promise Apple this server stays up. That promise is
this section.
