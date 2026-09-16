# Research: systemd-sysupdate transfer capabilities for GitHub Releases bundles

**Ticket:** [classy-giraffe/Ingot#12](https://github.com/classy-giraffe/Ingot/issues/12)
**Date:** 2026-09-16
**systemd version researched:** v261.3 (released 2026-09-10;
[release](https://github.com/systemd/systemd/releases/tag/v261.3)). The man pages cited below
are the v261.2 set published by freedesktop.org, which is the documentation for the current
stable series.
**Method:** primary sources only — the freedesktop.org/systemd man page set, the systemd
source tree on GitHub, the systemd.io documentation, the UAPI specifications, and GitHub's
first-party REST documentation. Where behavior was ambiguous in the man pages, the source code
was consulted and is linked inline.

## TL;DR

1. `url-directory` **does not exist**. The only URL source types are `url-file` and `url-tar`
   (systemd v261.3 source, `resource_type_table`). Plain-HTTPS asset URLs work directly,
   including GitHub Releases per-release download URLs, as long as `SHA256SUMS`
   (and `SHA256SUMS.gpg`) are uploaded as release assets.
2. sysupdate resolves "latest" **only** by extracting `@v` from the file *names* listed in the
   `SHA256SUMS` manifest at a **fixed** `Path=` URL and ordering them with a versionsort-style
   comparison. It parses no `manifest.json`, performs no HTTP directory listing, and follows
   only plain redirects. So "latest version from a manifest" is **not** a native capability —
   that is exactly what a thin oneshot helper must fill (together with authenticating
   `manifest.json` itself, which sysupdate cannot fetch or verify at all).
3. Natively verified (URL sources): GPG signature of the `SHA256SUMS` manifest (default on,
   keyring `/etc/systemd/import-pubring.pgp`, `/etc/systemd/import-pubring.gpg`,
   `/usr/lib/systemd/import-pubring.pgp`) **and** the SHA256 of every downloaded payload
   against the manifest (unconditional). Local `regular-file`/`directory` sources are
   transferred **without any integrity or authentication check**.
4. A/B + rollback: sysupdate never flips a boot entry. It installs versioned instances
   (payload partition with a versioned GPT label + versioned UKI file on `$BOOT` with the
   `+tries-left-tries-done` counters). `systemd-boot` orders entries (newest first, exhausted
   "bad" entries last) and decrements counters on each attempt; `systemd-bless-boot`
   (pulled in by its generator when boot counting is detected) strips the counters after a
   successful boot (`boot-complete.target`). Rollback is automatic: when the new version's
   tries-left reaches zero the entry is "bad" and the next-newest good entry is booted.

---

## 1. Transfer protocol capabilities

### 1.1 Resource types — no `url-directory`

`sysupdate.d(5)` defines the supported resource types
([man page, "Resource Types"](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)):

| Type | Source? | Compatible targets | Integrity + auth | Decompression |
|---|---|---|---|---|
| `url-file` | yes | `regular-file`, `partition` | yes | yes |
| `url-tar` | yes | `directory`, `subvolume` | yes | yes |
| `regular-file` | yes | `regular-file`, `partition` | **no** | yes |
| `partition` | no (target only) | — | — | — |
| `tar` | yes | `directory`, `subvolume` | **no** | yes |
| `directory` | yes | `directory`, `subvolume` | **no** | no |
| `subvolume` | yes | `directory`, `subvolume` | **no** | no |

The authoritative enumeration is the string table in the source,
[`resource_type_table`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c)
(`src/sysupdate/sysupdate-resource.c`), which contains exactly `url-file`, `url-tar`, `tar`,
`partition`, `regular-file`, `directory`, `subvolume`. **There is no `url-directory` type**, so
the premise of the ticket's second sub-question resolves to: *a URL source can never be a
directory tree* — a directory layout over HTTP is expressed either as a single `url-tar`
source (one tarball → one directory/subvolume) or as **multiple `url-file` transfers**, each
fetching one named file (this is how the foobarOS example in the man page bundles root +
verity + UKI).

### 1.2 What a URL source fetches: the `Path=` + `SHA256SUMS` scheme

From `sysupdate.d(5)`, `[Source] Path=`
([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)):

> If the source type is selected as `url-file` or `url-tar` this must be a HTTP/HTTPS URL.
> The URL is suffixed with `/SHA256SUMS` to acquire the manifest file, with
> `/SHA256SUMS.gpg` to acquire the detached signature file for it, and with the file names
> listed in the manifest file in case an update is executed and a resource shall be
> downloaded.

So a URL source is **entirely a fixed-path scheme**:

- manifest: `<Path>/SHA256SUMS`
- manifest signature: `<Path>/SHA256SUMS.gpg`
- payload: `<Path>/<name>` for each name in the manifest that matches the transfer's
  `MatchPattern=`

No HTML parsing, no JSON manifests, no directory listing of any kind. Confirmed in the source:
[`download_manifest()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c)
builds the manifest URL with `import_url_append_component(url, "SHA256SUMS")` and downloads it
via `systemd-pull raw --direct --verify <signature|no> <url> -`.

The URL construction is plain suffix appending with trailing-slash/query/fragment
normalization — `import_url_append_component()` =
`import_url_change_suffix(url, 0, suffix)` in
[`src/shared/import-util.h`](https://github.com/systemd/systemd/blob/main/src/shared/import-util.h)
/ [`src/shared/import-util.c`](https://github.com/systemd/systemd/blob/main/src/shared/import-util.c)
(`n_drop_components == 0` means *no* path component is removed; a single `/` is inserted and
the result ends in exactly one slash). Verified by executing a line-for-line replica of the
function on 2026-09-16:

```
https://host/files/                              -> https://host/files/SHA256SUMS
https://host/files                               -> https://host/files/SHA256SUMS
https://github.com/o/r/releases/download/v1.2.3  -> https://github.com/o/r/releases/download/v1.2.3/SHA256SUMS
https://github.com/o/r/releases/download/v1.2.3  -> https://github.com/o/r/releases/download/v1.2.3/ingot_1.2.3.root.xz
```

Consequences:

- **Single-file layout over plain HTTPS:** directly supported. `Path=` is the URL of the
  directory that contains the assets (trailing slash recommended); each asset is one
  `url-file` transfer. The systemd project's own test scenario
  (`test/units/TEST-72-SYSUPDATE.sh`,
  [source](https://github.com/systemd/systemd/blob/main/test/units/TEST-72-SYSUPDATE.sh))
  exercises exactly this shape with `Type=url-file`, `Path=file://$WORKDIR/source`,
  `SHA256SUMS` inside that directory, a `uki-@v.efi` → `EFI/Linux` UKI transfer with the
  `+@l-@d` boot-counting patterns, and an erofs payload transfer
  (`linux-@v.erofs`).
- **Directory layout over plain HTTPS:** supported two ways. (a) `url-tar`: one tarball asset
  unpacked into a `directory`/`subvolume` target. (b) Multiple `url-file` transfers for the
  individual files. The manifest parser additionally accepts *relative subpaths* as entries
  (e.g. `subdir/file`): [`resource_load_from_web()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c)
  accepts "either a plain filename or a relative subpath like `subdir/file`" while rejecting
  absolute paths, non-normalized paths, `%`, escapes, and control characters. Whether a given
  HTTP host actually serves sub-path names is the host's problem (on GitHub Releases an asset
  name is a flat segment after the tag).
- **`SHA256SUMS` format:** strict GNU `sha256sum(1)` output
  ([man page](https://man7.org/linux/man-pages/man1/sha256sum.1.html)): 64 hex chars, space,
  binary/text marker, filename, newline. The parser rejects NUL bytes, non-UTF-8, line
  escapes, and requires a final newline
  ([`resource_load_from_web()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c)).
  The man page recommends `--binary` mode and ASCII-only plain file names.
- **Freshness marker:** a `BEST-BEFORE-YYYY-MM-DD` entry in `SHA256SUMS` (must hash to the
  empty file) makes the whole listing invalid after that date
  ([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html);
  [`process_magic_file()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c),
  overridable with `SYSTEMD_SYSUPDATE_VERIFY_FRESHNESS=0`).

### 1.3 "Latest version" resolution

Version enumeration for a URL source is: parse `SHA256SUMS`, match each entry's *name*
against `MatchPattern=` (which must contain `@v`), and extract the version string from the
name
([`sysupdate.d(5)`, "Match Patterns"](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)).
Instances are then ordered newest-first with
`strverscmp_improved()` ([`src/fundamental/string-util.c`](https://github.com/systemd/systemd/blob/main/src/fundamental/string-util.c)):
a versionsort-style comparison with `-`/`.` as segment separators, `~` for pre-releases
(older), `^` for patched releases (newer), e.g.
`123~rc1-1 < 123-a.1 < 123-1 < 123^post1 < 124-1`.

Therefore:

- "Latest" = the lexicographically/versionsort-maximum `@v` **within one fixed `Path=`
  manifest**. There is no notion of "latest release", "latest tag", or any server-side
  version index.
- A `manifest.json` (Ingot's bundle descriptor) is invisible to sysupdate: it is not the
  manifest format, and nothing in the sysupdate/pull code parses JSON release metadata.
- Redirects *are* followed — downloads go through libcurl with
  `CURLOPT_FOLLOWLOCATION` enabled ([`src/shared/curl-util.c`](https://github.com/systemd/systemd/blob/main/src/shared/curl-util.c))
  — so a server that 302-redirects the manifest/asset URLs (including GitHub, see 1.4) is
  fine. But the *version* still comes out of the file name.

### 1.4 Fit with GitHub Releases (first-party behavior)

From GitHub's REST API documentation for releases
([docs.github.com](https://docs.github.com/en/rest/releases/releases)):

- Every release asset carries a `browser_download_url` — the direct
  `https://github.com/{owner}/{repo}/releases/download/{tag}/{asset}` URL.
- `GET /repos/{owner}/{repo}/releases/latest` resolves the latest release as "the most recent
  **non-prerelease, non-draft** release, sorted by the `created_at` attribute" — i.e.
  **date-based**, not version-based.
- Assets are only reachable by exact name (API listing or `browser_download_url`); there is no
  HTTP directory listing of a release's assets.

GitHub also exposes the web URL `https://github.com/{owner}/{repo}/releases/latest/download/{name}`.
Verified live on 2026-09-16 against `systemd/systemd` (whose latest release v261.3 has no
assets):

```
$ curl -sIL https://github.com/systemd/systemd/releases/latest/download/SHA256SUMS
HTTP/2 302
location: https://github.com/systemd/systemd/releases/download/v261.3/SHA256SUMS
HTTP/2 404
```

i.e. `latest/download/<name>` 302-redirects to the tagged asset URL and yields a plain 404
when the asset does not exist — both behaviors play well with libcurl redirect following and
with the 404-driven signature fallback (1.5).

**Workable GitHub Releases layouts** (bundle assets per release):

| Layout | `Path=` | Manifest asset | Works |
|---|---|---|---|
| per-tag (recommended) | `https://github.com/{owner}/{repo}/releases/download/{tag}` | `SHA256SUMS` (asset, exact name) | yes — the tag must be known/baked in |
| tracking "latest" | `https://github.com/{owner}/{repo}/releases/latest/download` | `SHA256SUMS` (asset of the latest release) | yes — via 302 redirect; "latest" is GitHub's date-based one |

The per-tag layout makes the release tag a *configuration input* to sysupdate (see §2, §3):
sysupdate itself cannot discover the tag. The `latest/download` layout removes that input but
substitutes GitHub's date-based "latest" for Ingot's `manifest.json`-driven notion.

### 1.5 Manifest signature resolution order (GitHub gotcha, benign)

When fetching the manifest with `--verify signature`, systemd-pull resolves the detached
signature by trying, in order, per-file then per-directory names, restarting on 404
([`pull_make_verification_jobs()` / `pull_job_restart_with_signature()`](https://github.com/systemd/systemd/blob/main/src/import/pull-common.c)):

1. `SHA256SUMS.sha256.asc` (404 on GitHub — not an asset)
2. `SHA256SUMS.sha256.gpg` (404 on GitHub — not an asset)
3. `SHA256SUMS.gpg` ← the per-directory signature; this is the one to ship as a release asset
4. (`SHA256SUMS.asc` in the final per-directory fallback)

Net effect: ship `SHA256SUMS` + `SHA256SUMS.gpg` as release assets and the two extra 404
probes are harmless. Payloads are then verified against the literal SHA256 taken from the
manifest (see §3).

---

## 2. Configuration shape for a two-provider setup

### 2.1 Where transfer definitions live

`sysupdate.d(5)`
([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)):

- `/etc/sysupdate.d/*.transfer`
- `/run/sysupdate.d/*.transfer`
- `/usr/local/lib/sysupdate.d/*.transfer`
- `/usr/lib/sysupdate.d/*.transfer`

Each `*.transfer` file defines **one transfer** with three sections: `[Transfer]`,
`[Source]`, `[Target]`. Multiple files with the same `@v` version form one combined update
(an "update set"); a version is offered as an update only if **every** enabled transfer has an
instance of that version — otherwise the version is skipped entirely ("the server wants to
send us an update with parts of the OS missing"), per
[`context_discover_update_sets_by_flag()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate.c)
and the man page's "Basic Mode of Operation".

Override mechanics (verified in source):

- **Same-name files: first directory wins.** `conf_files_list()` walks the directories in the
  order above and skips any file name already seen in an earlier directory ("Skipping
  overridden file"), see
  [`files_add()` in `src/basic/conf-files.c`](https://github.com/systemd/systemd/blob/main/src/basic/conf-files.c).
  So `/etc/sysupdate.d/10-payload.transfer` replaces `/usr/lib/sysupdate.d/10-payload.transfer`,
  and `/run/` replaces `/etc/`.
- **`--definitions=DIR`** loads transfers *only* from `DIR` instead of the standard
  directories ([`systemd-sysupdate(8)`](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html))
  — the cleanest switch for a dev/test provider.
- **`--component=NAME`** reads from `sysupdate.NAME.d/` directories instead — an alternative
  partitioning (the man page warns against components for resources that must update
  together, which is our case).
- **`--offline`** (v257+) stops fetching `SHA256SUMS` from the network; **`--verify=BOOL`**
  forces signature verification on/off (default on).

### 2.2 The two providers

- **Production provider (HTTPS / GitHub Releases).** Shipped in
  `/usr/lib/sysupdate.d/`. `[Source] Type=url-file`, `Path=` a GitHub Releases per-release
  download URL, versioned `MatchPattern=`. Integrity/auth as per §3.
- **Dev/test provider (local directory, staging at `/var/lib/sysupdate-cache`).** The same
  transfer *targets*, but `[Source] Type=regular-file` (or `directory`/`tar` for a tree) with
  `Path=/var/lib/sysupdate-cache`. Per the resource-type table, local sources get **no
  integrity or authentication verification** — acceptable for dev/test, not for production.
  (Note: `/var/lib/sysupdate-cache` is just Ingot's chosen staging path; it is not a
  sysupdate concept.)

Switching providers without touching the shipped files:

1. copy/symlink the staged `*.transfer` files into `/etc/sysupdate.d/` (same file names →
   override), or
2. run `systemd-sysupdate … --definitions=/var/lib/sysupdate-cache/sysupdate.d` (only the
   staged set is used).

The production `Path=` contains the release tag, which is not known at image-build time. The
recommended shape is therefore: the oneshot update helper resolves the version (from
`manifest.json`, see §3) and writes the *resolved* transfer definitions into
`/run/sysupdate.d/` (which overrides `/usr/lib/sysupdate.d/`), then invokes
`systemd-sysupdate`. The shipped `/usr/lib/sysupdate.d/` files serve as the template/default
(e.g. pointing at `releases/latest/download` if Ingot accepts GitHub's date-based latest, or
with a placeholder the helper always overrides).

### 2.3 Worked example

Bundle per release `v1.2.3` of `classy-giraffe/Ingot`, with these release assets (flat
names, no subpaths):

```
ingot_1.2.3.root.erofs          # erofs payload for the OS slot (spec §8: usr/root slot)
ingot_1.2.3.efi                 # UKI (spec §12.2)
SHA256SUMS                      # GNU sha256sum(1) listing of the two assets
SHA256SUMS.gpg                  # detached GPG signature of SHA256SUMS
manifest.json                   # Ingot bundle descriptor — helper-only, NOT in SHA256SUMS
```

(`manifest.json` is intentionally **not** listed in `SHA256SUMS`: sysupdate never fetches
names that no `MatchPattern=` matches, and keeping the manifest list limited to transfer
resources is the least surprising layout.)

**Shipped (production) transfers** — `/usr/lib/sysupdate.d/`:

```ini
# /usr/lib/sysupdate.d/10-payload.transfer
[Transfer]
ProtectVersion=%A

[Source]
Type=url-file
Path=https://github.com/classy-giraffe/Ingot/releases/latest/download
MatchPattern=ingot_@v.root.erofs

[Target]
Type=partition
Path=auto
MatchPattern=ingot_@v
MatchPartitionType=usr-x86-64
ReadOnly=1
InstancesMax=2
```

```ini
# /usr/lib/sysupdate.d/20-kernel.transfer
[Transfer]
ProtectVersion=%A

[Source]
Type=url-file
Path=https://github.com/classy-giraffe/Ingot/releases/latest/download
MatchPattern=ingot_@v.efi

[Target]
Type=regular-file
Path=/EFI/Linux
PathRelativeTo=boot
MatchPattern=ingot_@v+@l-@d.efi \
             ingot_@v+@l.efi \
             ingot_@v.efi
Mode=0644
TriesLeft=3
TriesDone=0
InstancesMax=2
```

Notes, all per the
[man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)
(following its foobarOS example):

- `ProtectVersion=%A` marks the running OS version (from `IMAGE_VERSION=` in
  `/etc/os-release`) as protected so it is never wiped/overwritten while booted.
- The payload transfer writes to the block device of the root filesystem (`Path=auto`,
  partition target) and matches slot partitions by GPT type `usr-x86-64` (spec §8.3);
  partition labels carry the version (`ingot_@v`), an `_empty` label marks a free slot.
  `InstancesMax=2` keeps at most two concurrent versions — exactly two A/B slots; the
  oldest slot is "emptied" (label set to `_empty`) when the next update starts.
- The UKI transfer installs into `$BOOT/EFI/Linux` (UAPI.1 Type #2 location); the three
  `MatchPattern=` variants are the Automatic Boot Assessment naming scheme — the UKI is
  installed as `ingot_1.2.3+3.efi` (`TriesLeft=3`), renamed by the boot loader through
  `+2-1`, `+1-2`, … and finally renamed by `systemd-bless-boot` to `ingot_1.2.3.efi` after a
  good boot. Ordering transfers by file name so the UKI (the boot entry point) is written
  last is the man page's explicit recommendation.
- `Path=…/releases/latest/download` makes the shipped config track GitHub's latest release
  natively (1.4). If Ingot requires `manifest.json` as the version authority, the helper
  rewrites the `[Source] Path=` of both files (into `/run/sysupdate.d/`) with the concrete
  per-tag URL `…/releases/download/{tag}` before invoking sysupdate.

**Dev/test (staging) transfers** — `/var/lib/sysupdate-cache/sysupdate.d/`, same targets,
local sources (staged bundle: `ingot_1.2.3.root.erofs`, `ingot_1.2.3.efi` copied into the
staging dir):

```ini
# /var/lib/sysupdate-cache/sysupdate.d/10-payload.transfer
[Transfer]
ProtectVersion=%A

[Source]
Type=regular-file
Path=/var/lib/sysupdate-cache
MatchPattern=ingot_@v.root.erofs

[Target]
Type=partition
Path=auto
MatchPattern=ingot_@v
MatchPartitionType=usr-x86-64
ReadOnly=1
InstancesMax=2
```

```ini
# /var/lib/sysupdate-cache/sysupdate.d/20-kernel.transfer
[Transfer]
ProtectVersion=%A

[Source]
Type=regular-file
Path=/var/lib/sysupdate-cache
MatchPattern=ingot_@v.efi

[Target]
Type=regular-file
Path=/EFI/Linux
PathRelativeTo=boot
MatchPattern=ingot_@v+@l-@d.efi \
             ingot_@v+@l.efi \
             ingot_@v.efi
Mode=0644
TriesLeft=3
TriesDone=0
InstancesMax=2
```

Local `regular-file` sources are decompressed if compressed (`.xz`/`.gz`/… supported), and
per the resource-type table carry **no** integrity/auth verification — fine for dev/test.

**The "sysupdate unit".** systemd has no `.sysupdate` unit type — the unit that drives an
update is a regular systemd service unit invoking the `systemd-sysupdate` tool
([`systemd-sysupdate(8)`](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html);
the companion daemon `systemd-sysupdated.service` exposes the same operations on D-Bus/Varlink
for unprivileged clients,
[man page](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdated.service.html)).
Example oneshot unit (dev/test; production would drop the `--definitions=`/`--verify=no`
pieces and typically run on a timer):

```ini
# /etc/systemd/system/ingot-sysupdate.service
[Unit]
Description=Apply Ingot OS update via systemd-sysupdate (dev/test provider)

[Service]
Type=oneshot
# Dev/test: use the staged local transfer definitions, skip signature verification.
ExecStart=/usr/lib/systemd/systemd-sysupdate update --definitions=/var/lib/sysupdate-cache/sysupdate.d --verify=no --reboot
```

```ini
# /etc/systemd/system/ingot-sysupdate.service  (production variant)
[Unit]
Description=Apply Ingot OS update via systemd-sysupdate

[Service]
Type=oneshot
# Helper has already resolved the version and written /run/sysupdate.d/*.transfer;
# pin the exact version resolved from manifest.json:
ExecStart=/usr/lib/systemd/systemd-sysupdate update 1.2.3 --reboot
```

Relevant verbs: `check-new` (exit 0 + version on stdout if an update exists), `list`,
`acquire` (download without installing, v260+), `update [VERSION]`, `pending` (newer
installed than running, per `IMAGE_VERSION=`), `reboot` (reboot if `pending`), `vacuum`
([`systemd-sysupdate(8)`](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html)).
`systemd-sysupdate-reboot.service` (timer-triggered) handles "installed but not yet booted"
reboots automatically in the no-helper design.

---

## 3. Native checksum/signature verification — and the gaps a helper must fill

### 3.1 What sysupdate verifies natively

For `url-file`/`url-tar` sources (the only source types with integrity+auth at all,
[man page resource table](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)):

1. **GPG signature of the `SHA256SUMS` manifest** — `Verify=` (default `yes`; CLI
   `--verify=`): "validate the GPG signatures for downloaded `SHA256SUMS` manifest files, via
   their detached signature files `SHA256SUMS.gpg` in combination with the system keyring
   `/usr/lib/systemd/import-pubring.pgp` or `/etc/systemd/import-pubring.pgp`"
   ([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)).
   Mechanically: the manifest is downloaded via
   `systemd-pull raw --direct --verify signature`
   ([`download_manifest()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c));
   the detached signature is resolved per §1.5; and
   [`verify_gpg()`](https://github.com/systemd/systemd/blob/main/src/import/pull-common.c)
   runs `gpg --batch --trust-model=always --verify <sig> -` against throwaway copies of the
   first keyring found among `$SYSTEMD_OPENPGP_KEYRING` (absolute path override),
   `/etc/systemd/import-pubring.pgp`, `/etc/systemd/import-pubring.gpg`,
   `/usr/lib/systemd/import-pubring.pgp` (paths from
   [`meson.build`](https://github.com/systemd/systemd/blob/main/meson.build)).
   Note `--trust-model=always`: the signing key must be *present in the keyring*; no trust
   chain/expiry evaluation is performed.
2. **SHA256 of each downloaded payload, unconditionally** — "the downloaded payload files are
   unconditionally checked against the SHA256 hashes listed in the manifest. This option
   [Verify=] only controls whether the signatures of these manifests are verified"
   ([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)).
   The sysupdate code passes the manifest's hash as a literal checksum to the downloader —
   `systemd-pull raw --direct --verify <sha256-hex> <url> <target>`
   ([`transfer_acquire_instance()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-transfer.c))
   — and refuses to download when the checksum is unknown.
3. **Manifest freshness** via optional `BEST-BEFORE-YYYY-MM-DD` (§1.2).
4. **Download atomicity/robustness** (not crypto, but relevant): URL → file/partition writes
   stage through `.sysupdate.partial.<version>` / `.sysupdate.pending.<version>` names and
   (for partitions) derived "partial"/"pending" GPT type UUIDs, with final `rename()` +
   fsync — an incomplete download is detectable and flushed on the next invocation
   ([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html);
   [`src/sysupdate/sysupdate-transfer.c`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-transfer.c),
   [`src/sysupdate/sysupdate-resource.c`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c)).

**What is NOT verified:**

- Local sources (`regular-file`, `directory`, `subvolume`, `tar`): "no integrity or
  authentication verification is done" (man page, resource types and `regular-file`
  description). This applies to the dev/test provider in §2.2.
- `manifest.json`: not a transfer resource, not the manifest format — sysupdate never
  downloads, parses, or checks it.
- Per-file signatures of the payloads (only the one manifest signature + per-file SHA256).
- Anything in the UKI (kernel/initrd signatures are a separate Secure Boot story, out of
  scope here).

### 3.2 Gaps the lightweight oneshot helper must fill

1. **Resolve the latest version from `manifest.json`.** sysupdate's "latest" is the
   `strverscmp_improved`-maximum `@v` inside the `SHA256SUMS` of the pinned `Path=`; it
   cannot read `manifest.json` or discover a release. The helper fetches `manifest.json`
   (plain HTTPS — e.g. as an asset of the latest release via
   `…/releases/latest/download/manifest.json`, or from a fixed URL) and derives the version
   and/or tag.
2. **Authentify `manifest.json` itself.** sysupdate has no check for it; the helper owns
   that trust decision (e.g. verify a signature, or anchor trust in the GitHub API/release
   provenance). Once the version is fixed, the *payload* trust chain remains sysupdate's
   native `SHA256SUMS.gpg` + SHA256 verification.
3. **Bind the resolved version to the transfers.** Either rewrite the `[Source] Path=` of the
   transfers (into `/run/sysupdate.d/`) with the concrete `…/releases/download/{tag}` URL, or
   simply invoke `systemd-sysupdate update <version>` after pointing at a manifest that
   offers that version. `update` accepts an explicit version and warns (but proceeds) when it
   is not the newest available or is older than the newest installed
   ([`verb_update()`](https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate.c)).
4. **Reconcile the two "latest" orderings.** GitHub's latest is date-based (most recent
   non-prerelease/non-draft by release-commit date, per the REST docs), sysupdate's is
   version-based (`strverscmp_improved` over filenames). They agree for well-behaved
   sequential releases and diverge otherwise (e.g. a backported `1.9.9` released after
   `2.0.0`, or `make_latest=legacy` semantics). If Ingot's `manifest.json` is the authority,
   the helper must not blindly follow `releases/latest`.
5. **(Optional) cross-check** `manifest.json` contents against the `SHA256SUMS` entries it
   implies, and pass `--offline`-style constraints (e.g. pinning) as needed.

**Headline:** the helper is needed, but narrowly — for *version discovery* (manifest.json
→ version/tag) and *manifest.json authentication*. Integrity/authenticity of the actual
update payloads, staging, installation, and slot management all remain native sysupdate
behavior behind a GPG-signed `SHA256SUMS`.

---

## 4. A/B slot switching and systemd-bless-boot

### 4.1 What sysupdate writes

- **Payload:** the erofs image is decompressed and written into a *partition* — one of the
  pre-existing slots of `MatchPartitionType=` (spec §8.2: `os-slot-a`/`os-slot-b`, 4–8 GiB).
  Partitions must pre-exist (create them with `systemd-repart`); the slot's GPT label is set
  to the version pattern (`ingot_@v`), `_empty` marks a free slot, and `InstancesMax=2` keeps
  exactly the two newest versions, emptying the older slot when a new update starts
  ([man page](https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html)).
  `ProtectVersion=%A` shields the booted slot.
- **UKI:** written as a versioned *regular file* on `$BOOT` (`EFI/Linux/ingot_<v>+<tries>.efi`),
  with `Mode=`, `TriesDone=`/`TriesLeft=` applied per the Automatic Boot Assessment scheme,
  and up to `InstancesMax=2` UKIs kept.

### 4.2 Who flips the boot entry

**Nobody in userspace explicitly flips it.** The selection is the boot loader's, driven by
file naming:

- `systemd-boot` orders entries newest-first and keeps entries whose boot counter
  (`+tries-left` in the filename, UAPI.1 boot counting) is still non-zero ahead of exhausted
  ones; on each attempt it decrements tries-left / increments tries-done by *renaming the
  entry file* (e.g. `ingot_1.2.3+2-1.efi`) before booting
  ([systemd.io, Automatic Boot Assessment](https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/);
  [UAPI.1 boot counting](https://uapi-group.org/specifications/specs/boot_loader_specification/#boot-counting)).
- A UKI pins its payload slot via the embedded kernel command line (root partition UUID), so
  "slot switch" = "boot the UKI whose command line points at the freshly written slot". The
  man page's foobarOS example shows how to keep payload/UKI consistent by fixing partition
  UUIDs (`@u` wildcard / `PartitionUUID=`).
- **Feedback loop:**
  - *Success:* `systemd-bless-boot.service` (pulled in by
    `systemd-bless-boot-generator` when systemd-boot-style boot counting is detected) runs
    after `boot-complete.target` — which `systemd-boot-check-no-failures.service` and any
    other checks gate — and strips the `+l-d` counters from the booted entry's filename
    (via the `LoaderBootCountPath` EFI variable), marking it permanently "good"
    ([systemd.io](https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/);
    [`systemd-bless-boot(8)`](https://www.freedesktop.org/software/systemd/man/latest/systemd-bless-boot.html)).
  - *Failure:* the loader keeps decrementing; when tries-left hits zero the entry is
    "bad" and ordered after all non-bad entries, so the next-newest entry (the previous
    slot's UKI) is booted automatically. `systemd-bless-boot bad` forces this state
    immediately; `systemd-bless-boot status|good|indeterminate` are the manual controls.
- **Post-update reboot:** `systemd-sysupdate pending`/`reboot` compare the newest installed
  version with `IMAGE_VERSION=` from `/etc/os-release` of the running system;
  `systemd-sysupdate-reboot.service` (timer) reboots into a completed-but-not-yet-booted
  update ([`systemd-sysupdate(8)`](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html)).

So the A/B model is: **two versioned instances coexisting on disk; the boot loader chooses
among them by version + boot counters; bless-boot closes the feedback loop.** Rollback needs
no sysupdate invocation at all.

### 4.3 Design implication for Ingot (SPECS.md §12–13)

SPECS.md's early boot-entry sketch uses *fixed* slot names (`os-slot-a.efi` /
`os-slot-b.efi`, [docs/SPECS.md](../SPECS.md) §12.2, untracked Draft 1.0 in the main
worktree). sysupdate's instance model is inherently **versioned** — it names what it
installs from `MatchPattern=` and manages versions, not fixed slot identifiers. The
mechanically consistent design is:

- payload slots: versioned GPT labels (`ingot_<v>`), type `usr-x86-64`, `InstancesMax=2`
  (A/B = "the two newest versions", slot identity emergent);
- UKIs: versioned filenames with boot counters (`ingot_<v>+<l>.efi`) — this is what makes
  `systemd-bless-boot` and automatic rollback work; fixed names `os-slot-{a,b}.efi` would
  disable the counter scheme and thus the automatic-rollback feedback loop;
- "slot switching" = boot-loader ordering, not a userspace flip; `systemd-sysupdate
  update` + reboot completes the cycle.

---

## Sources

- sysupdate.d(5) — https://www.freedesktop.org/software/systemd/man/latest/sysupdate.d.html
- systemd-sysupdate(8) — https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html
- systemd-sysupdated.service(8) — https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdated.service.html
- systemd-bless-boot(8) — https://www.freedesktop.org/software/systemd/man/latest/systemd-bless-boot.html
- Automatic Boot Assessment — https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/
- UAPI.1 Boot Loader Specification (boot counting) — https://uapi-group.org/specifications/specs/boot_loader_specification/#boot-counting
- UAPI.2 Discoverable Partitions Specification — https://uapi-group.org/specifications/specs/discoverable_partitions_specification
- sha256sum(1) — https://man7.org/linux/man-pages/man1/sha256sum.1.html
- systemd v261.3 release (2026-09-10) — https://github.com/systemd/systemd/releases/tag/v261.3
- src/sysupdate/sysupdate-resource.c (resource types, manifest download/parse, version sort) — https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-resource.c
- src/sysupdate/sysupdate-transfer.c (download callouts, literal-SHA256 verification, partial/pending staging) — https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate-transfer.c
- src/sysupdate/sysupdate.c (update-set discovery, candidate selection, `update` verb) — https://github.com/systemd/systemd/blob/main/src/sysupdate/sysupdate.c
- src/import/pull-common.c (signature fallback chain, keyrings, `verify_gpg`) — https://github.com/systemd/systemd/blob/main/src/import/pull-common.c
- src/shared/import-util.{h,c} (`Path=` URL suffix semantics) — https://github.com/systemd/systemd/blob/main/src/shared/import-util.c
- src/shared/curl-util.c (redirect following) — https://github.com/systemd/systemd/blob/main/src/shared/curl-util.c
- src/fundamental/string-util.c (`strverscmp_improved`) — https://github.com/systemd/systemd/blob/main/src/fundamental/string-util.c
- src/basic/conf-files.c (first-directory-wins override) — https://github.com/systemd/systemd/blob/main/src/basic/conf-files.c
- meson.build (keyring install paths) — https://github.com/systemd/systemd/blob/main/meson.build
- test/units/TEST-72-SYSUPDATE.sh (official sysupdate scenario) — https://github.com/systemd/systemd/blob/main/test/units/TEST-72-SYSUPDATE.sh
- GitHub REST API: releases — https://docs.github.com/en/rest/releases/releases
- Live verification (2026-09-16): `https://github.com/systemd/systemd/releases/latest/download/SHA256SUMS` → 302 `…/releases/download/v261.3/SHA256SUMS` → 404
- Ingot design reference: `docs/SPECS.md` (Draft 1.0), §8, §12, §13 — repo file
