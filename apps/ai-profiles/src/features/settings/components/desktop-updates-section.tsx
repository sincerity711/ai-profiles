import type { DesktopUpdate, DesktopUpdateAction } from '@/lib/types'

import { useCallback, useEffect, useState } from 'react'

import { Button } from '@/design'
import { type AppId, appSpecs } from '@/lib/app-registry'
import { desktopUpdateAction, getDesktopUpdates } from '@/lib/commands'
import { extractErrorMessage } from '@/lib/extract-error-message'

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
const displayVersion = (version: string | null, build: string | null) =>
  build && build !== version ? `${version} (build ${build})` : version

export function DesktopUpdatesSection() {
  const [apps, setApps] = useState<Array<DesktopUpdate> | null>(null)
  const [pending, setPending] = useState<Partial<Record<AppId, string>>>({})
  const [errors, setErrors] = useState<Partial<Record<AppId, string>>>({})
  const [loadError, setLoadError] = useState('')
  const load = useCallback(async () => {
    try {
      const rows = await getDesktopUpdates()
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

  async function act(app: AppId, action: DesktopUpdateAction) {
    setPending((p) => ({ ...p, [app]: action }))
    setErrors((p) => ({ ...p, [app]: '' }))
    try {
      await desktopUpdateAction(app, action)
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
        Official releases, checked every six hours while ai-profiles is open. Downloads use your saved proxy; retrying
        an interrupted download resumes it when supported. Install when all instances are closed; account data stays in
        place.
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
      {apps?.map((row) => (
        <DesktopUpdateRow key={row.app} row={row} pending={pending[row.app]} error={errors[row.app]} onAction={act} />
      ))}
    </section>
  )
}

type UpdateRowProps = {
  row: DesktopUpdate
  pending?: string
  error?: string | null
  onAction: (app: AppId, action: DesktopUpdateAction) => Promise<void>
}

function updateStatus(row: DesktopUpdate): string | undefined {
  if (row.busy || !row.readyVersion) return phases[row.phase]
  const version = displayVersion(row.readyVersion, row.readyBuild)
  return row.readyOutdated
    ? `Downloaded ${version} is older than the latest release. Download the latest update before installing.`
    : `Version ${version} is downloaded and verified. Install it to finish updating.`
}

function availableRelease(row: DesktopUpdate): string {
  if (!row.latestVersion) return 'Check for a compatible release'
  return row.updateAvailable
    ? `Version ${displayVersion(row.latestVersion, row.latestBuild)} available`
    : 'Latest compatible release installed'
}

function canInstall(row: DesktopUpdate): boolean {
  return Boolean(row.readyVersion) && !row.readyOutdated
}

function canDownload(row: DesktopUpdate): boolean {
  return Boolean(row.latestVersion) && !canInstall(row) && (row.updateAvailable || !row.managed)
}

function downloadLabel(row: DesktopUpdate, pending?: string): string {
  if (row.busy && row.phase === 'downloading') return 'Downloading…'
  if (row.busy && row.phase === 'verifying') return 'Verifying…'
  if (pending === 'download') return 'Preparing download…'
  if (row.readyOutdated) return 'Download latest update'
  return row.updateAvailable ? 'Download update' : 'Download managed copy'
}

function DesktopUpdateRow({ row, pending, error: actionError, onAction }: UpdateRowProps) {
  const name = appSpecs[row.app].displayName
  const working = row.busy || Boolean(pending)
  const error = actionError || row.error
  const status = updateStatus(row)
  return (
    <section className="border-t border-border-soft py-4" aria-label={`${name} updates`}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h4 className="font-medium text-ink">{name}</h4>
        <span className="text-sm tabular-nums text-muted">
          {row.installedVersion
            ? `Installed ${displayVersion(row.installedVersion, row.installedBuild)}`
            : 'Not installed'}
        </span>
      </div>
      <p className="mt-1 text-sm text-muted">
        {availableRelease(row)}
        {row.installedVersion && !row.managed ? ' · External installation' : ''}
      </p>
      <UpdateActions row={row} pending={pending} working={working} onAction={onAction} />
      {status && (
        <p role="status" className="mt-2 text-sm text-ink-soft">
          {status}
        </p>
      )}
      <UpdateProgress row={row} />
      <UpdateNotices row={row} error={error} />
    </section>
  )
}

function UpdateActions({ row, pending, working, onAction }: Omit<UpdateRowProps, 'error'> & { working: boolean }) {
  const installBlocked = working || row.running
  return (
    <div className="mt-3 flex flex-wrap gap-2">
      <Button size="sm" variant="secondary" disabled={working} onClick={() => void onAction(row.app, 'check')}>
        Check for updates
      </Button>
      {canDownload(row) && (
        <Button size="sm" disabled={working} onClick={() => void onAction(row.app, 'download')}>
          {downloadLabel(row, pending)}
        </Button>
      )}
      {canInstall(row) && (
        <Button size="sm" disabled={installBlocked} onClick={() => void onAction(row.app, 'install')}>
          Install {displayVersion(row.readyVersion, row.readyBuild)}
        </Button>
      )}
      {row.previousVersion && (
        <Button size="sm" variant="ghost" disabled={installBlocked} onClick={() => void onAction(row.app, 'rollback')}>
          Restore {displayVersion(row.previousVersion, row.previousBuild)}
        </Button>
      )}
    </div>
  )
}

function UpdateProgress({ row }: { row: DesktopUpdate }) {
  const name = appSpecs[row.app].displayName
  return (
    <>
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
    </>
  )
}

function UpdateNotices({ row, error }: Pick<UpdateRowProps, 'row' | 'error'>) {
  const name = appSpecs[row.app].displayName
  return (
    <>
      {row.running && (canInstall(row) || row.previousVersion) && (
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
    </>
  )
}
