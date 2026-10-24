# The WebUI package: `ui/main` at `/`, other UI packages at `/<name>/`

Since v0.4.0 the program has **no WebUI built in**. The WebUI is a directory of static files in the configuration root:
`<config root>/ui/main/` is served at `/`; any other `<config root>/ui/<name>/` is served at `/<name>/` (`ui/pg/` at `/pg/`, `ui/a/` at `/a/`).
Without `ui/main` the program is a pure backend: `/` says that no WebUI is installed, and the API works as always ([headless.md](headless.md)).
Contract: [native-webui-hosting.md](contracts/native-webui-hosting.md) ("Same-origin hosting", "WebUI packages").

**Directory names:** non-empty, no `/` or `\`, no control characters, a real directory (not a symlink), and not one of the reserved names
`root`, `log`, `log1`, `legacy`, `blog`, `assets`, `debug`, `metrics`, `plugins`, `api`, `chunks`. Anything else is listed as unusable in health and is not served.
`main` is served at `/` only (not at `/main/`).

## Build a package (developer)
```sh
cd webui-log
UI_VERSION=v0.4.0-ui2 npm run build            # webui-log/dist/ (UI_VERSION is optional; default: git describe)
UI_VERSION=v0.4.0-ui2 npm run build:package    # and release/rtdns-webui.tar.gz
```
`webui-log/dist/` holds `index.html`, `app.js`, `app.css`, `chunks/`, `assets/` and `ui-manifest.json`; `rtdns-webui.tar.gz` has the same files at its top level
(the release asset of the same name is built this way). Resource URLs are **relative**, so one package works as `ui/main` and under any other name.

## Install / update (operator)
Online updates replace `ui/main` together with the program when `ui/main` exists ([native-update.md](contracts/native-update.md)); by hand:
1. Unpack into the directory (the configuration root is the directory that holds `config_custom.yaml`; for `rtdns start -d /cus/mosdns` that is `/cus/mosdns`):
   ```sh
   mkdir -p /cus/mosdns/ui/main && tar -xzf rtdns-webui.tar.gz -C /cus/mosdns/ui/main      # the WebUI at /
   mkdir -p /cus/mosdns/ui/pg && tar -xzf rtdns-webui.tar.gz -C /cus/mosdns/ui/pg          # another copy at /pg/
   ```
   Replace a package by unpacking into an empty directory (remove the old one first), so no files of the old version are left behind.
2. **Restart once** so the program checks the package (`POST /api/v1/system/restart`, the "重启" button, or `systemctl restart rtdns`). Directories are registered at start-up.
3. Open `http://<host>:<api port>/` (or `/pg/`). The system page shows the WebUI's version as 界面版本 and links to every other mounted package.

Replacing files of an already mounted package takes effect on the next request (files are re-read live). A *new* package version's manifest is judged — and its version shown in
`GET /api/v1/system/health` → `external_ui` — only at the next start, so restart once after every manual update.

## Which directories are "mosdns UI packages"
Only a directory that contains `ui-manifest.json` is treated as a mosdns UI package and checked for compatibility. A directory **without** a manifest is served as plain static
pages (so other static pages you keep under `ui/` are unaffected); health lists it as `kind: "static"` with the note "非 mosdns UI 包（无 manifest）".

## When a package does not fit
`ui-manifest.json` says which interface level the UI was built for. If it is broken, or the UI needs a newer interface than this program provides (or is older than the program still
supports), or `index.html` is missing, the package is **not mounted**: `/<name>/` is a 404 (for `ui/main`, `/` shows the "no WebUI" page with the reason), and
`GET /api/v1/system/health` shows `{"name": "pg", "path": "/pg/", "mounted": false, "kind": "mosdns_ui", "reason": "…"}`. Fix by installing a matching package or updating the program.

## Roll back
Unpack the previous version's `rtdns-webui.tar.gz` into the directory (and restart once). Deleting `ui/main` leaves the pure backend; deleting another `ui/<name>` removes only that address.
