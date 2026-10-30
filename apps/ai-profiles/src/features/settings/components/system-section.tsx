import type { StatusTone } from '@/design'
import type { Dependencies, Shell } from '@/lib/types'

import { useEffect, useState } from 'react'

import { RotateCw } from 'lucide-react'

import { Button, Skeleton, StatusDot } from '@/design'
import { useAppMetadata } from '@/features/about/api/use-app-metadata'
import { useDependencies } from '@/features/dependencies/api/use-dependencies'
import { hookInstallMessage, rcDisplay, shellHookStatus } from '@/features/settings/lib/system-status'
import { appIds, appSpecs } from '@/lib/app-registry'
import { detectShell, installPathHook } from '@/lib/commands'

type Row = {
  label: string
  tone: StatusTone
  detail: string
  aux?: string
}

// Version strings aren't surfaced from the Rust side yet (see 99-todo.md).
// Until then we render an em-dash next to each installed dependency — the
// status dot already conveys installed vs. not-detected.
const MISSING_DETAIL = '—'

const REFRESH_FLASH_MS = 1500

function buildAppRows(dependencies: Dependencies): Array<Row> {
  return appIds.flatMap((id) => {
    const spec = appSpecs[id]
    const deps = dependencies.apps[id]
    return [
      { label: `${spec.displayName} Desktop`, tone: deps.guiInstalled ? 'success' : 'warning', detail: MISSING_DETAIL },
      {
        label: `${spec.cliDisplayName} CLI`,
        tone: deps.cliInstalled ? 'success' : 'warning',
        detail: MISSING_DETAIL,
      },
    ]
  })
}

function buildRows(
  deps: Dependencies,
  shell: Shell | null,
  updater: { tone: StatusTone; detail: string },
  version: string,
): Array<Row> {
  return [
    ...buildAppRows(deps),
    {
      label: 'Shell PATH',
      tone: deps.localBinOnPath ? 'success' : 'warning',
      detail: shell ? rcDisplay[shell] : MISSING_DETAIL,
    },
    {
      label: 'Launcher build',
      tone: updater.tone,
      detail: updater.detail,
      aux: `v${version}`,
    },
  ]
}

/**
 * Consolidated System status card.
 *
 * Rows: two per managed app (Desktop / CLI) from the registry, plus Shell
 * PATH and launcher build — six rows in total when both Claude and ChatGPT are
 * registered. Each row shows a status dot (success/warning/neutral), a
 * label, and a mono detail string. Beneath the card a hookline lets the
 * user re-install the shell hook in one click and an updater hookline lets
 * them identify this local custom build. Desktop updates have their own section.
 *
 * The section owns its own data: `useDependencies` suspends here (not at
 * the SettingsView level) so the rest of the Settings pane can paint
 * instantly while this card resolves.
 */
export function SystemSection() {
  const dependencies = useDependencies()
  const { version } = useAppMetadata()
  const [shell, setShell] = useState<Shell | null>(null)

  useEffect(() => {
    void detectShell().then(setShell)
  }, [])

  async function refresh() {
    await Promise.all([dependencies.refresh(), detectShell().then(setShell)])
  }

  const updaterDescription = { tone: 'neutral' as const, detail: 'Local custom build' }
  const rows = buildRows(dependencies.deps, shell, updaterDescription, version)

  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">System</span>
        <RefreshControl onRefresh={refresh} />
      </div>
      <div className="rounded-xl border border-border bg-white py-1 dark:bg-cream-2">
        {rows.map((row, index) => (
          <StatusRow
            // biome-ignore lint/suspicious/noArrayIndexKey: rows are a stable ordered list with no insert/reorder semantics
            key={index}
            row={row}
          />
        ))}
      </div>
      <ShellHookControls
        localBinOnPath={dependencies.deps.localBinOnPath}
        shell={shell}
        onInstalled={dependencies.refresh}
      />
      <p className="mt-2 text-sm text-muted">
        Upstream launcher updates are disabled to preserve your proxy and app management features.
      </p>
    </section>
  )
}

type RefreshControlProps = {
  /**
   * Re-reads the dependencies and the shell; resolves once both are back.
   */
  onRefresh: () => Promise<unknown>
}

/**
 * The header's Refresh button, spinning while a refresh runs and followed by a
 * brief "Refreshed" confirmation once it lands.
 */
function RefreshControl({ onRefresh }: RefreshControlProps) {
  const [refreshing, setRefreshing] = useState(false)
  const [refreshedAt, setRefreshedAt] = useState<number | null>(null)

  useEffect(() => {
    if (refreshedAt === null) {
      return
    }
    const handle = window.setTimeout(() => setRefreshedAt(null), REFRESH_FLASH_MS)
    return () => window.clearTimeout(handle)
  }, [refreshedAt])

  async function handleRefresh() {
    if (refreshing) {
      return
    }
    setRefreshing(true)
    setRefreshedAt(null)
    try {
      await onRefresh()
      setRefreshedAt(Date.now())
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <div className="flex items-center gap-2">
      {refreshedAt !== null ? (
        <span className="font-mono text-[11px] text-muted-strong" role="status" aria-live="polite">
          Refreshed
        </span>
      ) : null}
      <Button
        size="sm"
        variant="secondary"
        leadingIcon={<RotateCw className={`h-3.5 w-3.5 ${refreshing ? 'animate-spin' : ''}`} strokeWidth={1.85} />}
        disabled={refreshing}
        onClick={() => void handleRefresh()}
      >
        {refreshing ? 'Refreshing…' : 'Refresh'}
      </Button>
    </div>
  )
}

type StatusRowProps = {
  /**
   * The dependency or subsystem the row reports on.
   */
  row: Row
}

/**
 * One line of the System card: a status dot, a label, and a mono detail.
 */
function StatusRow({ row }: StatusRowProps) {
  return (
    <div className="grid grid-cols-[7px_1fr_auto] items-center gap-3 border-b border-border-soft px-4 py-3 text-[13px] tracking-[-0.003em] text-ink-soft last:border-b-0">
      <StatusDot pulse tone={row.tone} />
      <span>
        {row.label}
        {row.aux ? <span className="ml-2 font-mono text-[11.5px] text-muted">{row.aux}</span> : null}
      </span>
      <span className="font-mono text-[11.5px] text-muted">{row.detail}</span>
    </div>
  )
}

type ShellHookControlsProps = {
  /**
   * Whether `~/.local/bin` is on the user's PATH.
   */
  localBinOnPath: boolean
  /**
   * The detected shell, or `null` while detection runs.
   */
  shell: Shell | null
  /**
   * Re-reads the dependencies once the hook has been written.
   */
  onInstalled: () => Promise<unknown>
}

/**
 * The shell hookline: which shell was detected and whether its rc file has
 * the PATH hook, with a one-click (re-)install and its outcome underneath.
 */
function ShellHookControls({ localBinOnPath, shell, onInstalled }: ShellHookControlsProps) {
  const [hookMessage, setHookMessage] = useState<string | null>(null)
  const [hookError, setHookError] = useState<string | null>(null)
  const [hookBusy, setHookBusy] = useState(false)

  const hookInstalled = localBinOnPath && shell !== null

  async function handleReinstall() {
    if (!shell) {
      return
    }
    setHookBusy(true)
    setHookMessage(null)
    setHookError(null)
    try {
      const outcome = await installPathHook(shell)
      setHookMessage(hookInstallMessage(shell, outcome))
      await onInstalled()
    } catch (caught) {
      setHookError(caught instanceof Error ? caught.message : String(caught))
    } finally {
      setHookBusy(false)
    }
  }

  return (
    <>
      <div className="mt-2.5 flex items-center gap-2.5 font-mono text-[11px] text-muted-strong">
        <span>{shellHookStatus(shell, hookInstalled)}</span>
        <Button size="sm" variant="ghost" disabled={!shell || hookBusy} onClick={handleReinstall}>
          {hookInstalled ? 'Re-install hook' : 'Install hook'}
        </Button>
      </div>
      {hookMessage ? (
        <p role="status" aria-live="polite" className="mt-1 text-[11.5px] text-muted-strong">
          {hookMessage}
        </p>
      ) : null}
      {hookError ? (
        <p role="alert" className="mt-1 text-[11.5px] text-red">
          {hookError}
        </p>
      ) : null}
    </>
  )
}

/**
 * Skeleton placeholder for the System section while `useDependencies`
 * resolves. Kept colocated so the section's loading shape stays in sync
 * with its rendered shape.
 */
export function SystemSectionFallback() {
  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">System</span>
        <Skeleton className="h-7 w-[88px] rounded-md" />
      </div>
      <Skeleton className="h-[190px] w-full rounded-xl" />
      <Skeleton className="mt-2.5 h-4 w-[280px] rounded-sm" />
    </section>
  )
}
