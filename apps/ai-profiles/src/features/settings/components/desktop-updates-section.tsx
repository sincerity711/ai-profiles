import { useCallback, useEffect, useState } from 'react'

import { invoke } from '@tauri-apps/api/core'

import { Button } from '@/design'
import { type AppId, appSpecs } from '@/lib/app-registry'
import { extractErrorMessage } from '@/lib/extract-error-message'

export type DesktopUpdate = {
  app: AppId
  installedVersion: string | null
  installedPath: string | null
  managed: boolean
  latestVersion: string | null
  updateAvailable: boolean
  readyVersion: string | null
  previousVersion: string | null
  running: boolean
  busy: boolean
  phase: string
  downloaded: number
  total: number | null
  checkedAt: string | null
  error: string | null
}

const phases: Record<string, string> = {
  checking: 'Checking official releases…',
  downloading: 'Downloading…',
  verifying: 'Verifying signature and preparing the app…',
  installing: 'Installing…',
  'rolling-back': 'Restoring previous version…',
  installed: 'Installed. Profiles will rebuild from this version when next opened.',
  restored: 'Previous version restored. Open a profile to use it.',
}
const mb = (bytes: number) => `${Math.round(bytes / 1024 / 1024)} MB`

export function DesktopUpdatesSection() {
  const [apps, setApps] = useState<Array<DesktopUpdate> | null>(null)
  const [pending, setPending] = useState<Partial<Record<AppId, string>>>({})
  const [errors, setErrors] = useState<Partial<Record<AppId, string>>>({})
  const [loadError, setLoadError] = useState('')
  const load = useCallback(async () => {
    try {
      const rows = await invoke<Array<DesktopUpdate>>('get_desktop_updates')
      setApps(Array.isArray(rows) ? rows : [])
      setLoadError('')
    } catch (error) {
      setLoadError(extractErrorMessage(error))
    }
  }, [])
  useEffect(() => {
    void load()
    const timer = setInterval(() => void load(), 1500)
    return () => clearInterval(timer)
  }, [load])

  async function act(app: AppId, action: string) {
    setPending((p) => ({ ...p, [app]: action }))
    setErrors((p) => ({ ...p, [app]: '' }))
    try {
      await invoke('desktop_update_action', { app, action })
    } catch (error) {
      setErrors((p) => ({ ...p, [app]: extractErrorMessage(error) }))
    } finally {
      await load()
      setPending((p) => ({ ...p, [app]: '' }))
    }
  }

  return (
    <section className="mb-8" aria-labelledby="desktop-updates-heading">
      <h3 id="desktop-updates-heading" className="mb-2 font-semibold text-ink">
        Desktop app updates
      </h3>
      <p className="mb-4 text-sm text-muted">
        Official releases, checked every six hours while ai-profiles is open. Downloads use your saved proxy. Install
        when all instances are closed; account data stays in place.
      </p>
      {loadError && (
        <p role="alert" className="mb-3 text-sm text-red">
          {loadError}{' '}
          <Button size="sm" variant="ghost" onClick={() => void load()}>
            Retry
          </Button>
        </p>
      )}
      {!apps && !loadError && (
        <p role="status" className="text-sm text-muted">
          Loading installed apps…
        </p>
      )}
      {apps?.map((row) => {
        const name = appSpecs[row.app].displayName
        const working = row.busy || Boolean(pending[row.app])
        const error = errors[row.app] || row.error
        const status = row.busy
          ? phases[row.phase]
          : row.readyVersion
            ? `Version ${row.readyVersion} is verified and ready to install.`
            : phases[row.phase]
        const installBlocked = working || row.running
        return (
          <section key={row.app} className="border-t border-border-soft py-4" aria-label={`${name} updates`}>
            <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
              <h4 className="font-medium text-ink">{name}</h4>
              <span className="text-sm tabular-nums text-muted">
                {row.installedVersion ? `Installed ${row.installedVersion}` : 'Not installed'}
              </span>
            </div>
            <p className="mt-1 text-sm text-muted">
              {row.latestVersion
                ? row.updateAvailable
                  ? `Version ${row.latestVersion} available`
                  : 'Latest compatible release installed'
                : 'Check for a compatible release'}
              {row.installedVersion && !row.managed ? ' · External installation' : ''}
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              <Button size="sm" variant="secondary" disabled={working} onClick={() => void act(row.app, 'check')}>
                Check for updates
              </Button>
              {row.latestVersion && !row.readyVersion && (row.updateAvailable || !row.managed) && (
                <Button size="sm" disabled={working} onClick={() => void act(row.app, 'download')}>
                  {pending[row.app] === 'download'
                    ? 'Preparing download…'
                    : row.updateAvailable
                      ? 'Download update'
                      : 'Download managed copy'}
                </Button>
              )}
              {row.readyVersion && (
                <Button size="sm" disabled={installBlocked} onClick={() => void act(row.app, 'install')}>
                  Install {row.readyVersion}
                </Button>
              )}
              {row.previousVersion && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={installBlocked}
                  onClick={() => void act(row.app, 'rollback')}
                >
                  Restore {row.previousVersion}
                </Button>
              )}
            </div>
            {status && (
              <p role="status" className="mt-2 text-sm text-ink-soft">
                {status}
              </p>
            )}
            {row.busy && row.phase === 'downloading' && (
              <div className="mt-2">
                <progress
                  className="h-1.5 w-full accent-ink"
                  aria-label={`${name} download progress`}
                  value={row.total ? row.downloaded : undefined}
                  max={row.total ?? undefined}
                />
                <p className="mt-1 text-sm tabular-nums text-muted">
                  {mb(row.downloaded)}
                  {row.total ? ` / ${mb(row.total)}` : ''}
                </p>
              </div>
            )}
            {row.running && (row.readyVersion || row.previousVersion) && (
              <p className="mt-2 text-sm text-muted">
                Quit {name} and all its profile instances to install or restore. Your download is kept.
              </p>
            )}
            {row.installedVersion && !row.managed && (
              <p className="mt-2 text-sm text-muted">
                Installing creates an ai-profiles managed copy. Your current application is kept.
              </p>
            )}
            {row.installedPath && (
              <details className="mt-2 text-sm text-muted">
                <summary className="cursor-pointer">App location</summary>
                <p className="mt-1 break-all select-text">{row.installedPath}</p>
              </details>
            )}
            {error && (
              <p role="alert" className="mt-2 text-sm text-red">
                {error}
              </p>
            )}
          </section>
        )
      })}
    </section>
  )
}
