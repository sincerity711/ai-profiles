import type { ProxySettings } from '@/lib/types'

import { useCallback, useEffect, useState } from 'react'

import { Button } from '@/design'
import { Input } from '@/design/ui/input'
import { getDesktopProxy, saveDesktopProxy, testDesktopProxy } from '@/lib/commands'
import { extractErrorMessage } from '@/lib/extract-error-message'

export function ProxySection() {
  const [settings, setSettings] = useState<ProxySettings | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [error, setError] = useState(false)
  const load = useCallback(async () => {
    try {
      setSettings(await getDesktopProxy())
      setError(false)
      setMessage('')
    } catch (e) {
      setError(true)
      setMessage(extractErrorMessage(e))
    }
  }, [])
  useEffect(() => {
    void load()
  }, [load])
  async function run(command: 'save_desktop_proxy' | 'test_desktop_proxy') {
    if (!settings) return
    setBusy(true)
    setMessage('')
    try {
      await (command === 'save_desktop_proxy' ? saveDesktopProxy(settings) : testDesktopProxy(settings))
      setError(false)
      setMessage(
        command === 'save_desktop_proxy' ? 'Saved. Relaunch desktop profiles to apply.' : 'Proxy port is reachable.',
      )
    } catch (e) {
      setError(true)
      setMessage(extractErrorMessage(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="mb-8" aria-labelledby="proxy-heading">
      <h3 id="proxy-heading" className="mb-3 font-semibold text-ink">
        Desktop proxy
      </h3>
      <p className="mb-3 text-sm text-muted">
        Shared by managed desktop profiles. The proxy is checked before launch, including profiles with a distinct Dock
        icon.
      </p>
      {settings ? (
        <fieldset disabled={busy} className="space-y-3">
          <label className="flex items-center gap-2 text-sm text-ink">
            <input
              type="checkbox"
              checked={settings.enabled}
              onChange={(e) => setSettings({ ...settings, enabled: e.target.checked })}
            />
            Use HTTP proxy
          </label>
          <label className="block text-sm text-ink" htmlFor="desktop-proxy-url">
            Proxy URL
          </label>
          <Input
            id="desktop-proxy-url"
            value={settings.url}
            placeholder="http://host:3128"
            disabled={!settings.enabled}
            onChange={(e) => setSettings({ ...settings, url: e.target.value })}
          />
          <label className="block text-sm text-ink" htmlFor="desktop-proxy-bypass">
            Connect directly to (comma-separated hosts)
          </label>
          <Input
            id="desktop-proxy-bypass"
            value={settings.noProxy}
            placeholder="localhost,127.0.0.1,::1"
            disabled={!settings.enabled}
            onChange={(e) => setSettings({ ...settings, noProxy: e.target.value })}
          />
          <div className="flex gap-2">
            <Button size="sm" disabled={busy || !settings.enabled} onClick={() => void run('test_desktop_proxy')}>
              Test connection
            </Button>
            <Button size="sm" disabled={busy} onClick={() => void run('save_desktop_proxy')}>
              {busy ? 'Working…' : 'Save proxy'}
            </Button>
          </div>
        </fieldset>
      ) : error ? (
        <Button size="sm" onClick={() => void load()}>
          Retry
        </Button>
      ) : (
        <p>Loading proxy settings…</p>
      )}
      {message && (
        <p className="mt-3 text-sm text-ink" role={error ? 'alert' : 'status'}>
          {message}
        </p>
      )}
    </section>
  )
}
