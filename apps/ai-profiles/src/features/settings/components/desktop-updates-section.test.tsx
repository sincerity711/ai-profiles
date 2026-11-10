import { invoke } from '@tauri-apps/api/core'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'

import { type DesktopUpdate, DesktopUpdatesSection } from './desktop-updates-section'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const row: DesktopUpdate = {
  app: 'claude',
  installedVersion: '1.0',
  installedBuild: '1.0',
  installedPath: '/managed/Claude.app',
  managed: true,
  latestVersion: '2.0',
  latestBuild: '2.0',
  updateAvailable: true,
  readyVersion: null,
  readyBuild: null,
  readyOutdated: false,
  previousVersion: null,
  previousBuild: null,
  running: true,
  busy: false,
  phase: 'checked',
  downloaded: 0,
  total: null,
  checkedAt: null,
  error: null,
}
beforeEach(() => vi.mocked(invoke).mockReset())

it('replaces an outdated download with the latest release while the app is running', async () => {
  let downloaded = false
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'desktop_update_action') {
      downloaded = true
      return
    }
    return [
      {
        ...row,
        app: 'codex',
        latestVersion: '26.1103.60933',
        latestBuild: '25894',
        readyVersion: downloaded ? '26.1103.60933' : '26.1027.61615',
        readyBuild: downloaded ? '25894' : '24141',
        readyOutdated: !downloaded,
      },
    ] as never
  })
  render(<DesktopUpdatesSection />)
  const download = await screen.findByRole('button', { name: 'Download latest update' })
  expect(download).toBeEnabled()
  expect(screen.getByText(/is older than the latest release/)).toBeVisible()
  expect(screen.queryByRole('button', { name: /Install/ })).not.toBeInTheDocument()
  fireEvent.click(download)
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('desktop_update_action', { app: 'codex', action: 'download' }),
  )
  expect(await screen.findByRole('button', { name: 'Install 26.1103.60933 (build 25894)' })).toBeDisabled()
  expect(screen.getByText(/Quit ChatGPT and all its profile instances/)).toBeVisible()
})

it('shows download progress instead of preparation while the action is still pending', async () => {
  let downloading = false
  let finish: (() => void) | undefined
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'desktop_update_action') {
      downloading = true
      await new Promise<void>((resolve) => {
        finish = resolve
      })
      return
    }
    return [
      {
        ...row,
        busy: downloading,
        phase: downloading ? 'downloading' : 'checked',
        downloaded: 1024 * 1024,
        total: 373 * 1024 * 1024,
      },
    ] as never
  })
  render(<DesktopUpdatesSection />)
  fireEvent.click(await screen.findByRole('button', { name: 'Download update' }))
  expect(await screen.findByRole('button', { name: 'Preparing download…' })).toBeDisabled()
  expect(await screen.findByRole('button', { name: 'Downloading…' }, { timeout: 3000 })).toBeDisabled()
  expect(screen.getByText('1 MB / 373 MB')).toBeVisible()
  downloading = false
  finish?.()
  await waitFor(() => expect(screen.getByRole('button', { name: 'Download update' })).toBeEnabled())
})

it('downloads explicitly without installing or quitting a running app', async () => {
  vi.mocked(invoke).mockResolvedValue([row])
  render(<DesktopUpdatesSection />)
  fireEvent.click(await screen.findByRole('button', { name: 'Download update' }))
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('desktop_update_action', { app: 'claude', action: 'download' }),
  )
  expect(vi.mocked(invoke).mock.calls.some(([, args]) => args && 'action' in args && args.action === 'install')).toBe(
    false,
  )
})

it('keeps a verified download and blocks installation while instances are running', async () => {
  vi.mocked(invoke).mockResolvedValue([
    { ...row, readyVersion: '2.0' },
    { ...row, app: 'codex', readyVersion: '2.0' },
  ])
  render(<DesktopUpdatesSection />)
  const group = await screen.findByRole('region', { name: 'ChatGPT updates' })
  expect(within(group).getByRole('button', { name: 'Install 2.0' })).toBeDisabled()
  expect(within(group).getByText(/Quit ChatGPT and all its profile instances/)).toBeVisible()
})

it('installs a staged release when closed and shows actionable failures', async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'desktop_update_action') throw 'Signature verification failed; download again'
    return [{ ...row, readyVersion: '2.0', running: false }] as never
  })
  render(<DesktopUpdatesSection />)
  fireEvent.click(await screen.findByRole('button', { name: 'Install 2.0' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('Signature verification failed')
  await waitFor(() => expect(screen.getByRole('button', { name: 'Install 2.0' })).not.toBeDisabled())
})

it('distinguishes installed and downloaded builds with the same display version', async () => {
  vi.mocked(invoke).mockResolvedValue([
    {
      ...row,
      app: 'codex',
      installedVersion: '26.1027.61615',
      installedBuild: '24140',
      latestVersion: '26.1027.61615',
      latestBuild: '24141',
      readyVersion: '26.1027.61615',
      readyBuild: '24141',
    },
  ])
  render(<DesktopUpdatesSection />)
  expect(await screen.findByText('Installed 26.1027.61615 (build 24140)')).toBeVisible()
  expect(screen.getByText('Version 26.1027.61615 (build 24141) available')).toBeVisible()
  expect(screen.getByText(/is downloaded and verified. Install it to finish updating/)).toBeVisible()
  expect(screen.getByRole('button', { name: 'Install 26.1027.61615 (build 24141)' })).toBeDisabled()
})

it('removes the update offer when installation reports the current build', async () => {
  let installed = false
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'desktop_update_action') {
      installed = true
      return
    }
    return [
      {
        ...row,
        running: false,
        installedVersion: installed ? '2.0.0' : '1.0',
        installedBuild: installed ? '2.0.0' : '1.0',
        updateAvailable: !installed,
        readyVersion: installed ? null : '2.0',
        readyBuild: installed ? null : '2.0',
        phase: installed ? 'installed' : 'ready',
      },
    ] as never
  })
  render(<DesktopUpdatesSection />)
  fireEvent.click(await screen.findByRole('button', { name: 'Install 2.0' }))
  expect(await screen.findByText('Latest compatible release installed')).toBeVisible()
  expect(screen.getByText('Installed 2.0.0')).toBeVisible()
  expect(screen.queryByRole('button', { name: /Download update|Install 2.0/ })).not.toBeInTheDocument()
})
