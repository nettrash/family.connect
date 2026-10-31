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

## 4. Encryption at rest

*Since #59 (2026-10-25).* What people wrote, said, showed or are is encrypted
by the server before it reaches PostgreSQL or the disk (docs/protocol.md,
"Stored encrypted"; the design is `docs/encryption-at-rest-2026-10-24.md`).
A dump, a backup, a snapshot or a stolen disk opens nothing without **the key
file**. It protects copies, not the running box: whoever has root there can
read what the server reads, exactly as before.

### The key file

One file, 32 random bytes in base64. Every family, conversation and account
has its own key inside the database, and this file is what unlocks them. The
server **refuses to start** without it or with the wrong one, and says which
fingerprint it expected; it never falls back to storing plaintext.

```sh
sudo family-connect storage-key new /etc/family-connect/storage.key   # refuses to overwrite
sudo chown family-connect:family-connect /etc/family-connect/storage.key
sudo chmod 0400 /etc/family-connect/storage.key
```

```toml
# /etc/family-connect/config.toml
[storage]
key_file = "/etc/family-connect/storage.key"
```

> **Copy it off the box now, once, by hand** — into a password manager, or
> printed and put away: `sudo family-connect storage-key show`. Without it
> nothing the server ever stored can be opened again: not the database, not a
> backup, not one photograph. It is in **no** backup, deliberately — a backup
> that carried its own key would protect nothing.

**Stronger: systemd's encrypted credentials.** The key then never sits on disk
in the clear, and with a TPM it opens only on this machine:

```sh
sudo systemd-creds encrypt --name=storage.key \
  /etc/family-connect/storage.key /etc/credstore.encrypted/storage.key
sudo systemctl edit family-connect   # add the two lines below
#   [Service]
#   LoadCredentialEncrypted=storage.key:/etc/credstore.encrypted/storage.key
# and in config.toml:
#   key_file = "/run/credentials/family-connect.service/storage.key"
sudo shred -u /etc/family-connect/storage.key   # only after the server starts with the credential
```

A TPM-bound credential cannot be opened on new hardware, so the off-box copy
is what a rebuilt server is restored with. Keep it.

### Upgrading a server that has data

The first start of a version with this section converts everything already
stored. Do it in this order:

1. **Take a backup** with the old version (section 5). It holds plaintext and
   will be deleted in step 6, but it is the only way back: **after the
   conversion, the older version cannot read the database.**
2. Make the key file and copy it off the box, as above.
3. Install the new binary, the new backup script (section 5) and the
   `key_file` line, and restart. Before it answers anything the server seals
   the database (seconds for one family; the log counts rows, never names
   them), then compacts the converted tables so the old plaintext rows leave
   the table files. Then it serves, and seals the attachment files in the
   background — `journalctl -u family-connect` shows the count. An interrupted
   conversion continues where it stopped on the next start.
4. Check: `sudo -u postgres psql -d family_connect -tAc "SELECT substr(body, 1, 4) FROM messages LIMIT 3"`
   shows `\x01…`, not words. Then open the app and look at a photo.
5. Run a backup with the new script and check that it wrote the key copy.
6. **Delete every backup from before the upgrade** — restic snapshots, rsync
   mirrors, local dumps, a WAL archive if you keep one. They hold plaintext and
   always will. Free disk blocks keep old bytes until they are overwritten,
   which is why an encrypted volume underneath (LUKS) is still worth having.

### Rotating the key

```sh
sudo systemctl stop family-connect
sudo family-connect storage-key rotate   # shows the NEW key: copy it off the box
sudo systemctl start family-connect
```

`rotate` writes the new key beside the old one, and in one transaction
re-wraps the per-family keys under it and recomputes the lookups that are
keyed by it — one per username, invite code and device token. Only then does
it move the old file to `storage.key.old` and the new one into its place,
owned as the old one was. Nothing that was sealed is touched, so it takes a
moment however much the family has stored. With an encrypted credential,
decrypt it to a file first (`systemd-creds decrypt`), rotate that, and encrypt
the new one in its place.

**Keep the old key for 7 days**: the key copies in your backups from before the
rotation are wrapped by it (section 5). After that, destroy it.

### Reading the assistant reports

A member's report about the assistant comes to you, not to the family owner
(docs/protocol.md, "Reporting the assistant"), and the server logs a WARN
line when one arrives. The row is sealed like everything else, so `psql` shows
bytes; read them with:

```sh
sudo -u family-connect family-connect reports            # the newest 50
sudo -u family-connect family-connect reports --limit 200
```

A report about a conversation that has since been deleted shows its reason and
date, and `(gone with its conversation)` where the reply was — deleting a
conversation takes its words from the reports too.

### What deletion means now

Deleting a family, a group, a direct or assistant chat, or an account destroys
its key. The backup keeps key copies for **7 days**, so a week later no backup
can open what was deleted, even with the key file. A single message, note or
file deleted on its own (by its author or by retention) is not covered; older
backups hold it until they expire.

---

## 5. Backups

**A database dump is not a backup.** Attachment bytes live on disk, not in
PostgreSQL, so a `pg_dump` alone restores a family's history with every
photograph missing. Both halves, or neither.

**And since #59 there is a third part: the keys.** The script leaves the
per-family keys out of the main dump (`--exclude-table-data=storage_keys`) and
writes them in the same run to a separate key copy, kept **7 days** — restic
under the tag `family-connect-keys`, rsync under `keys/`, pruned to the same
7 days. That separation is the whole point of a key per family: if the keys
travelled with the data, the backup kept for a year would open every family
deleted during that year. Do not put the keys back into the main dump, and do
not run a plain `pg_dump` of the live database by hand and keep it. The key
FILE is in no backup at all (section 4).

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

## 6. Restoring

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

*Since #59* a restore needs two more things: **the key file**, from your
off-box copy (section 4), and **a key copy** — the one with the same stamp as
the dump, or, when that one has expired, the newest there is.

```sh
sudo systemctl stop family-connect

# --- half 1: the database, as postgres ---
DUMP=$(ls -t /mnt/offbox/family-connect/db/db-*.dump | head -1)   # or: restic restore
sudo -u postgres dropdb --if-exists family_connect
sudo -u postgres createdb -O family-connect family_connect
sudo -u postgres pg_restore --dbname=family_connect --no-owner --no-privileges "$DUMP"

# --- the keys, as postgres: the copy with the dump's stamp, else the newest ---
KEYS=/mnt/offbox/family-connect/keys/keys-$(basename "$DUMP" .dump | cut -d- -f2).dump
[ -f "$KEYS" ] || KEYS=$(ls -t /mnt/offbox/family-connect/keys/keys-*.dump | head -1)
sudo -u postgres pg_restore --dbname=family_connect --data-only --no-owner --no-privileges "$KEYS"

# --- half 2: the bytes, as family-connect ---
sudo install -d -o family-connect -g family-connect /var/lib/family-connect/attachments
sudo -u family-connect rsync -a \
  /mnt/offbox/family-connect/attachments/ /var/lib/family-connect/attachments/

# --- the key file, from your off-box copy, if this is a new box ---
#     (section 4: owner family-connect, mode 0400)

sudo systemctl start family-connect
```

**A key copy newer than the dump** — because the dump is older than 7 days —
is expected, and it means what was deleted after the dump stays deleted. The
server finds the rows whose key no longer exists and **refuses to start**,
naming how many families, conversations and accounts they are. Deleting them
again is one command, and it is yours to run, because it is the one step of a
restore that removes data:

```sh
sudo -u family-connect family-connect storage forget-missing
```

A key copy **older** than the dump would leave families created in between
without keys; the same refusal names them, and the fix is a newer key copy,
not `forget-missing`.

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

## 7. Monitoring

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

---

## 8. The web client

A browser client lives in `web/` and is served **by this same nginx, from
this same origin** — which is not a deployment convenience but the reason
there is no CORS anywhere on this wire (docs/protocol.md, "A browser is a
client too").

Build it and put it where nginx looks:

```bash
cd web && trunk build --release     # writes web/dist
sudo rsync -a --delete web/dist/ /var/www/family-connect/
```

`server/nginx/family-connect.conf` already has the `location /` that serves
it. A server running the phone apps alone REPLACES that block with
`location / { return 404; }`, which is what every non-API path got before
the web client existed — commenting it out is not the same, because nginx
then serves its own default page.

Merge it into the live file by hand rather than copying the repo's over it:
the live one has certbot's TLS lines in it. And if the live server block sets
an `add_header` of its own — HSTS, typically — repeat it inside the two
nested locations under `location /`: nginx does not inherit a server-level
`add_header` into a location that sets any `add_header` itself, so
`index.html` and the bundle would be served without it.

Two things about caching, and they are not symmetrical. Trunk puts a content
hash in the name of the WASM bundle and its JS, so those may be cached for a
year — the name changes when the bytes do. `index.html` names the current
bundle and must NOT be cached, or a browser holding a stale copy asks for a
bundle that is no longer on disk and gets a blank page. The config sets both.

**Nothing about the server changes for a browser.** It registers no device,
so it takes no push and needs no APNs or FCM key; its socket is the same
socket the apps hold, and its session is an ordinary session with the same
sliding expiry. The one operational difference is that a browser's token
lives in `sessionStorage`, so closing the tab ends the session for that
reader — which is what makes a shared machine safe and is worth saying to a
family that asks why the web keeps signing them out and the phone does not.


## 9. Paid families

`[subscriptions]` lets this server host families for an App Store
subscription (docs/protocol.md, "Paid families"; issue #89). It is off by
default, and a server that never turns it on has nothing to do here.

The subscription is sold by the app's publisher, so this section is for the
publisher's server. Turning it on anywhere else costs that server's families
money that goes to somebody else.

### Before turning it on

1. **App Store Connect → the app → Subscriptions.** Create one subscription
   group ("Family hosting") holding the products named in `product_ids`: by
   default `me.nettrash.FamilyConnect.family.monthly` and `….family.yearly`.
   Give them prices, a display name and a description, and the review
   screenshot Apple asks for.
2. **Billing Grace Period** (in the same place): turn it on, 16 days. While
   Apple retries a failed payment the subscription stays active, and the
   server counts it as active. The family's 14 days start only after that.
3. **App Information → App Store Server Notifications.** Set the URL to
   `https://<your server>/api/v1/appstore/notifications` for BOTH Production
   and Sandbox, version 2. The repo's nginx config already passes it
   through; nothing else needs a change there.
4. **App Privacy.** Add "Purchases", linked to the user, used for app
   functionality. The server keeps, per subscriber, Apple's transaction id,
   the product, the environment and when it expires. Say the same on the
   privacy policy page.
5. **Ship the app that can buy.** Until the iPhone/iPad/Mac app with the
   subscription screen is on the App Store, nobody can start a family once
   the switch is on.

### Turning it on

Add the section, keep the defaults unless you mean to change them, and
restart:

```toml
[subscriptions]
enabled = true
```

Every family that exists at that moment stays FREE — yours, and any other
already here — so nothing changes for anybody who is already using the
server. See them, and move one if you mean to:

```sh
sudo family-connect families list
sudo family-connect families plan 7 paid     # or: free
```

`list` prints names, so it prints to your terminal only. It never goes to
the journal. A family moved to `paid` whose owner has no subscription starts
its 14 days at the next hourly sweep, and every member is told. Moving one to
`free` stops the clock.

### What to watch

The journal says, with ids and never names:

- `subscription stored` / `subscription refused (<reason>)` — an app handing
  over a purchase. A refusal names the check that failed (the chain, a
  marker extension, the bundle, the product, the environment, the account
  token). Every refusal from a real app is a bug worth reporting.
- `app store notification applied` / `ignored` / `refused`. A burst of
  refusals is somebody posting junk at the URL, which costs nothing. Apple's
  own notifications should never be refused.
- `family unpaid`, `family covered again`, `family deleted (unpaid)`.

A server that was down for a while misses Apple's notifications for that
time (Apple retries five times over several days). Nothing is lost: the
owner's app hands over the current state on its next launch, and the 14 days
exist to give it time to.

### Testing it

Use a Sandbox tester (App Store Connect → Users and Access → Sandbox) on a
development or TestFlight build. Its monthly subscription renews every few
minutes and stops by itself after a few renewals, so you can see the whole
cycle in an afternoon:

1. the purchase;
2. the family starting;
3. the subscription expiring;
4. the banner and the push;
5. the deletion.

For step 5, shorten `unpaid_days` to 1 while you watch, and put it back.

### Turning it off

`enabled = false` and restart. Plans are kept but nothing reads them: no
limits, no deadlines, and nobody is deleted. Families created while it was
on keep their `paid` plan, and any deadline is dropped at that restart: a
family that was unpaid gets a fresh 14 days, with fresh warnings, if the
switch is turned on again. Move any you mean to keep free first.
