import { invoke } from '@tauri-apps/api/core'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'

import { ProxySection } from './proxy-section'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const settings = { enabled: true, url: 'http://127.0.0.1:3128', noProxy: 'localhost' }
beforeEach(() => vi.mocked(invoke).mockReset())

it('saves edited proxy settings without starting a profile', async () => {
  vi.mocked(invoke).mockResolvedValueOnce(settings).mockResolvedValue(undefined)
  render(<ProxySection />)
  fireEvent.change(await screen.findByLabelText('Proxy URL'), { target: { value: 'http://127.0.0.1:8080' } })
  fireEvent.click(screen.getByRole('button', { name: 'Save proxy' }))
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('save_desktop_proxy', {
      settings: { ...settings, url: 'http://127.0.0.1:8080' },
    }),
  )
  expect(await screen.findByRole('status')).toHaveTextContent('Saved')
})

it('shows connection failures and leaves settings editable', async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'test_desktop_proxy') throw { message: 'Proxy is unreachable' }
    return (command === 'get_desktop_proxy' ? settings : undefined) as never
  })
  render(<ProxySection />)
  fireEvent.click(await screen.findByRole('button', { name: 'Test connection' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('Proxy is unreachable')
  expect(screen.getByRole('button', { name: 'Save proxy' })).not.toBeDisabled()
})
