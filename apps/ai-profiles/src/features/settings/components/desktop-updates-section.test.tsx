import { invoke } from '@tauri-apps/api/core'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'

import { type DesktopUpdate, DesktopUpdatesSection } from './desktop-updates-section'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const row: DesktopUpdate = {
  app: 'claude',
  installedVersion: '1.0',
  installedPath: '/managed/Claude.app',
  managed: true,
  latestVersion: '2.0',
  updateAvailable: true,
  readyVersion: null,
  previousVersion: null,
  running: true,
  busy: false,
  phase: 'checked',
  downloaded: 0,
  total: null,
  checkedAt: null,
  error: null,
}
beforeEach(() => vi.mocked(invoke).mockReset())

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
