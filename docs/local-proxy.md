# Local desktop proxy build

This branch adds Settings → Desktop proxy. It stores the HTTP proxy and bypass
hosts in the app data directory as desktop-proxy.json. Each new managed desktop
launch checks the proxy TCP port before passing Chromium arguments and proxy
environment variables. Relaunch existing windows after changing settings. This
is a connectivity preflight, not a network firewall or traffic watchdog.

Generated plain launchers call the host with --launch-profile-direct, so
Finder/Dock launches use current settings too. Leave Distinct Dock icon off
when using the proxy. No vendor app is patched or re-signed.

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
