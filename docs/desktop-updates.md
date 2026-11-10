# Managed desktop updates

Settings → Desktop app updates manages Claude and the ChatGPT desktop app
formerly named Codex (bundle ID com.openai.codex). It does not install the
separate legacy ChatGPT app. Check automatically runs at most once every six
hours while ai-profiles is running. Download, install, and restore are explicit
actions. The UI reports actual versions, progress, running-app blocks and errors.

## Sources and verification

- Claude: the stable Squirrel feed on releases.claude.com; a locally generated
  updater UUID, OS version and version 0.0.0 request a full release. Archives
  come from downloads.claude.ai and must match the feed's SHA-256 and size.
- ChatGPT/Codex: persistent.oaistatic.com/codex-app-prod/appcast.xml. Choose the
  highest compatible build for this architecture and OS, never a delta. Verify
  the archive using the official Sparkle Ed25519 key before extraction.
- Both: require HTTPS and approved redirect hosts, then verify bundle identity,
  official Developer ID team, strict deep code signature, version, architecture
  and minimum OS. The official application is not patched or re-signed.

Official OpenAI references, retrieved directly over HTTPS on 2026-10-30:

- https://developers.openai.com/codex/app/
- https://developers.openai.com/codex/enterprise/manage-app-updates

The official macOS download links to the codex-app-prod distribution. The
installed official app confirms the appcast URL, bundle ID, signer and archive
verification key. Its current implementation supports CODEX_SPARKLE_ENABLED=false;
managed launches set this value to keep their updater off. This is an observed
implementation option, not the documented organization policy. The documented
managed-policy route is features.in_app_updates=false in requirements.toml.
External/Finder launches of the user's original app are not modified by this
integration. Imported Claude profiles retain disableAutoUpdates=true in their
native policy. New, non-imported Claude profiles need their own updater policy.

## Storage and recovery

All update data is under ~/Library/Application Support/ai-profiles/desktop-updates/.
Each app has a cross-process operation lock, status.json, a verified ready/
bundle and a previous/ bundle. Installation targets vendor/Claude.app or
vendor/ChatGPT.app. An existing /Applications app is retained; its initial
version is copied into the rollback slot when adopting a managed copy.

Installation and rollback refuse to run while any corresponding desktop
instance or helper is active. They do not quit applications, change profile
data, or restart this chat. A staged replacement is re-verified, then swapped
on the same filesystem. If the final rename fails, the previous app is restored.
A transaction journal restores an interrupted swap on the next status/action.
An older backup is retired only after the new rollback slot is in place.
Profile wrappers rebuild lazily using upstream's existing version detection.
Their ad-hoc signatures and Cowork limitations are unchanged.

Rolling back an application does not roll back profile data or undo data-format
migrations performed by the newer application. The UI retains the previous
binary; compatibility with data written by a newer vendor release is not guaranteed.

The custom ai-profiles build has no upstream updater plugin or update prompt.
Rebuild this fork to update the launcher without losing these changes.

## Verification

Rust tests cover compatible full-feed selection, source validation, archive
corruption, running process detection, failed replacement recovery, crash
recovery and cross-process locking. UI tests cover explicit downloads, blocking
installation while running and displaying recoverable errors.

Headless diagnostics use:

    ai-profiles --desktop-update status|check|download|install|rollback claude|codex

Use the UI for normal operation. Retrying a failed download resumes the partial
archive in each app's `download/` directory, including after restarting the app.
The saved release URL, version, size and checksum/signature must still match.
The server's byte range and total size are checked before appending; if it ignores
or refuses ranges, the download restarts safely. A failed checksum/signature
discards the corrupt archive so the next retry can fetch a clean copy.

Downloaded and verified means ready to install, not already installed. Status
reads the actual managed bundle after installation. Update checks compare the
feed build with `CFBundleVersion`; display versions and build numbers are shown
separately when they differ (as with ChatGPT). Numeric formatting differences
such as `1.0` and `1.0.0` are treated as equal during comparison and verification,
while a higher build of the same display version is still offered as an update.

If a check finds a newer build than the verified download, the UI offers
Download latest update instead of installing the outdated package. The backend
also refuses that outdated installation. The old verified package is retained
until the replacement is verified; downloading remains available while desktop
instances are running. Install and restore still require all instances and
their Renderer/Service helpers to exit. Installed refers to the managed vendor
bundle; profile copies rebuild from it the next time they are opened.
