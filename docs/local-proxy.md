# Local desktop proxy build

This branch adds Settings → Desktop proxy. It stores the HTTP proxy and bypass
hosts in the app data directory as desktop-proxy.json. Each new managed desktop
launch checks the proxy TCP port before passing Chromium arguments and proxy
environment variables. Relaunch existing windows after changing settings. This
is a connectivity preflight, not a network firewall or traffic watchdog.

Generated plain launchers call the host with --launch-profile-direct, so
Finder/Dock launches use current settings too. Distinct Dock icon wrappers
ask the host for current proxy settings and a connectivity preflight each time
their shim starts, including direct Finder/Dock launches. Failed preparation
stops the launch. Wrappers predating this support are rebuilt on opening from
ai-profiles. The official vendor bundle is not patched or re-signed; distinct
Dock copies retain upstream's ad-hoc signing and Cowork folder limitations.

Imported claude-multiprofile copies carry multiprofile-import.json. Only for
these copies, launch aligns the native egress policy and Code settings env
with the saved proxy. The native disableAutoUpdates policy is preserved.
Desktop auth is copied with gui-data; Code credentials in the path-derived
Keychain entry may require another login. Original profile directories remain.

Local import on 2026-10-30:

- claude1 → friday (Friday reset)
- claude2 → saturday (Saturday reset)

Build with pnpm 9.12.3 using:

```sh
pnpm --filter ai-profiles tauri build --debug --no-sign --bundles app
```

Keep changes on the local branch or a personal fork; origin belongs to the
original author. Installing an upstream ai-profiles release will not contain
this custom proxy feature.

## Official app location

The resolver prefers `~/Library/Application Support/ai-profiles/vendor/`
before `/Applications`. This Mac keeps the official Claude.app in that vendor
directory, intact and with its original signature. Friday and saturday share
the executable, with separate profile data. Managed launchers resolve the path
through ai-profiles and do not need regeneration.

To undo the move, quit Claude and move vendor/Claude.app back to
/Applications/Claude.app without overwriting another install. The resolver
falls back to that location. Old claude-multiprofile launchers that hardcode
/Applications/Claude.app must be updated or retired.

This relocation does not implement downloading or replacing vendor updates.
