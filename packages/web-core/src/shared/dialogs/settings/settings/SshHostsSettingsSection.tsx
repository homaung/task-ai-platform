import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowClockwiseIcon,
  CheckCircleIcon,
  CodeIcon,
  KeyIcon,
  PlugIcon,
  TerminalWindowIcon,
  XIcon,
} from '@phosphor-icons/react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import type {
  SshConnectionInfo,
  SshHostsResponse,
  SshHostSummary,
  SshLaunchTool,
} from 'shared/types';

import { sshHostsApi } from '@/shared/lib/api';
import { openLocalApiWebSocket } from '@/shared/lib/localApiTransport';
import { getTerminalTheme } from '@/shared/lib/terminalTheme';
import { getResolvedTheme, useTheme } from '@/shared/hooks/useTheme';
import { cn } from '@/shared/lib/utils';

const buttonClass =
  'inline-flex items-center justify-center gap-2 rounded border border-border bg-secondary px-base py-2 text-sm text-normal hover:text-high disabled:cursor-not-allowed disabled:opacity-50';
const primaryButtonClass = `${buttonClass} border-brand bg-brand text-white hover:text-white`;

function decodeBase64(value: string): string {
  const binary = atob(value);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

function encodeBase64(value: string): string {
  const bytes = new TextEncoder().encode(value);
  const binary = Array.from(bytes, (byte) => String.fromCodePoint(byte)).join(
    ''
  );
  return btoa(binary);
}

function hostDestination(host: SshHostSummary): string {
  const hostname = host.user ? `${host.user}@${host.hostname}` : host.hostname;
  return host.port && host.port !== 22 ? `${hostname}:${host.port}` : hostname;
}

function toolInstalled(info: SshConnectionInfo, tool: SshLaunchTool): boolean {
  return tool === 'shell' || info.tools.some((item) => item.name === tool);
}

function SshTerminal({
  alias,
  path,
  tool,
  onClose,
}: {
  alias: string;
  path: string;
  tool: SshLaunchTool;
  onClose: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const { theme } = useTheme();

  useEffect(() => {
    if (!containerRef.current) return;

    const terminal = new Terminal({
      cursorBlink: true,
      fontSize: 12,
      fontFamily: '"IBM Plex Mono", monospace',
      theme: getTerminalTheme(),
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon());
    terminal.open(containerRef.current);
    fitAddon.fit();
    terminal.focus();

    let socket: WebSocket | null = null;
    let disposed = false;
    const query = new URLSearchParams({
      alias,
      path,
      tool,
      cols: String(terminal.cols),
      rows: String(terminal.rows),
    });

    void openLocalApiWebSocket(`/api/ssh/terminal/ws?${query.toString()}`)
      .then((nextSocket) => {
        if (disposed) {
          nextSocket.close();
          return;
        }
        socket = nextSocket;
        socket.onmessage = (event) => {
          try {
            const message = JSON.parse(String(event.data)) as {
              type: 'output' | 'error';
              data?: string;
              message?: string;
            };
            if (message.type === 'output' && message.data) {
              terminal.write(decodeBase64(message.data));
            } else if (message.type === 'error' && message.message) {
              terminal.writeln(`\r\n\x1b[31m${message.message}\x1b[0m`);
            }
          } catch {
            terminal.writeln('\r\n\x1b[31mInvalid terminal response\x1b[0m');
          }
        };
        socket.onclose = () => terminal.writeln('\r\n[SSH session closed]');
      })
      .catch((reason: unknown) => {
        terminal.writeln(
          `\r\n\x1b[31m${reason instanceof Error ? reason.message : String(reason)}\x1b[0m`
        );
      });

    const dataSubscription = terminal.onData((data) => {
      if (socket?.readyState === WebSocket.OPEN) {
        socket.send(
          JSON.stringify({ type: 'input', data: encodeBase64(data) })
        );
      }
    });
    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit();
      if (socket?.readyState === WebSocket.OPEN) {
        socket.send(
          JSON.stringify({
            type: 'resize',
            cols: terminal.cols,
            rows: terminal.rows,
          })
        );
      }
    });
    resizeObserver.observe(containerRef.current);

    return () => {
      disposed = true;
      resizeObserver.disconnect();
      dataSubscription.dispose();
      socket?.close();
      terminal.dispose();
    };
  }, [alias, path, tool]);

  useEffect(() => {
    // A remount is unnecessary when only the application theme changes.
    const element = containerRef.current;
    if (element) element.style.colorScheme = getResolvedTheme(theme);
  }, [theme]);

  return (
    <section className="overflow-hidden rounded border border-border bg-primary">
      <header className="flex items-center gap-2 border-b border-border bg-secondary px-base py-2">
        <TerminalWindowIcon className="size-icon-sm text-brand" weight="bold" />
        <span className="min-w-0 flex-1 truncate font-mono text-sm text-high">
          {alias}:{path} · {tool}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="rounded p-1 text-low hover:bg-primary hover:text-high"
          aria-label="Close SSH terminal"
        >
          <XIcon className="size-icon-sm" />
        </button>
      </header>
      <div ref={containerRef} className="h-80 w-full px-2 py-1" />
    </section>
  );
}

export function SshHostsSettingsSection() {
  const [snapshot, setSnapshot] = useState<SshHostsResponse | null>(null);
  const [selectedAlias, setSelectedAlias] = useState<string | null>(null);
  const [connection, setConnection] = useState<SshConnectionInfo | null>(null);
  const [remotePath, setRemotePath] = useState('');
  const [loading, setLoading] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [terminal, setTerminal] = useState<{
    alias: string;
    path: string;
    tool: SshLaunchTool;
    key: number;
  } | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await sshHostsApi.list();
      setSnapshot(next);
      setSelectedAlias((current) => {
        if (current && next.hosts.some((host) => host.alias === current)) {
          return current;
        }
        return next.hosts[0]?.alias ?? null;
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selectedHost = useMemo(
    () => snapshot?.hosts.find((host) => host.alias === selectedAlias) ?? null,
    [selectedAlias, snapshot?.hosts]
  );

  const connect = useCallback(async () => {
    if (!selectedAlias) return;
    setConnecting(true);
    setError(null);
    setConnection(null);
    setTerminal(null);
    try {
      const info = await sshHostsApi.inspect(selectedAlias);
      setConnection(info);
      setRemotePath(info.repositories[0] ?? info.home_dir);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setConnecting(false);
    }
  }, [selectedAlias]);

  const launch = (tool: SshLaunchTool) => {
    if (!selectedAlias || !remotePath.trim()) return;
    setTerminal({
      alias: selectedAlias,
      path: remotePath.trim(),
      tool,
      key: Date.now(),
    });
  };

  return (
    <div className="space-y-6 pb-6">
      <section className="space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-medium text-high">SSH servers</h3>
            <p className="mt-1 text-sm text-low">
              Choose an existing Host from ~/.ssh/config. No key or tunnel
              command needs to be typed.
            </p>
          </div>
          <button
            type="button"
            className={buttonClass}
            onClick={() => void refresh()}
            disabled={loading}
          >
            <ArrowClockwiseIcon
              className={cn('size-icon-sm', loading && 'animate-spin')}
            />
            Refresh
          </button>
        </div>

        {snapshot?.config_path && (
          <p className="break-all rounded bg-secondary px-base py-2 font-mono text-xs text-low">
            {snapshot.config_path}
          </p>
        )}
        {error && (
          <div className="rounded border border-error/40 bg-error/10 px-base py-2 text-sm text-error">
            {error}
          </div>
        )}
        {!loading && snapshot && !snapshot.ssh_available && (
          <p className="text-sm text-error">OpenSSH client was not found.</p>
        )}
        {!loading && snapshot?.hosts.length === 0 && (
          <p className="rounded border border-border bg-secondary p-base text-sm text-low">
            No concrete Host entries were found in ~/.ssh/config.
          </p>
        )}

        <div className="grid gap-2 md:grid-cols-2">
          {snapshot?.hosts.map((host) => (
            <button
              type="button"
              key={host.alias}
              onClick={() => {
                setSelectedAlias(host.alias);
                setConnection(null);
                setTerminal(null);
              }}
              className={cn(
                'rounded border p-base text-left transition-colors',
                selectedAlias === host.alias
                  ? 'border-brand bg-brand/10'
                  : 'border-border bg-panel hover:bg-secondary'
              )}
            >
              <div className="flex items-center gap-2">
                <PlugIcon
                  className={cn(
                    'size-icon-sm',
                    selectedAlias === host.alias ? 'text-brand' : 'text-low'
                  )}
                  weight="bold"
                />
                <span className="font-medium text-high">{host.alias}</span>
              </div>
              <p className="mt-1 truncate font-mono text-xs text-low">
                {hostDestination(host)}
              </p>
              {host.identity_files.slice(0, 2).map((identityFile) => (
                <p
                  key={identityFile}
                  className="mt-1 flex items-center gap-1 truncate font-mono text-xs text-low"
                >
                  <KeyIcon className="size-icon-xs shrink-0" />
                  {identityFile}
                </p>
              ))}
            </button>
          ))}
        </div>

        {selectedHost && (
          <button
            type="button"
            className={primaryButtonClass}
            onClick={() => void connect()}
            disabled={connecting}
          >
            <PlugIcon
              className={cn('size-icon-sm', connecting && 'animate-pulse')}
            />
            {connecting
              ? `Connecting to ${selectedHost.alias}…`
              : `Connect to ${selectedHost.alias}`}
          </button>
        )}
      </section>

      {connection && (
        <section className="space-y-4 border-t border-border pt-5">
          <div className="flex items-center gap-2 text-success">
            <CheckCircleIcon className="size-icon-sm" weight="fill" />
            <span className="text-sm font-medium">
              Connected to {connection.alias}
            </span>
          </div>

          <div className="flex flex-wrap gap-2">
            {['git', 'claude', 'codex'].map((name) => {
              const installed = connection.tools.some(
                (tool) => tool.name === name
              );
              return (
                <span
                  key={name}
                  className={cn(
                    'rounded border px-2 py-1 font-mono text-xs',
                    installed
                      ? 'border-success/40 bg-success/10 text-success'
                      : 'border-border bg-secondary text-low'
                  )}
                >
                  {name}: {installed ? 'ready' : 'not found'}
                </span>
              );
            })}
          </div>

          <label className="block space-y-2">
            <span className="text-sm font-medium text-normal">
              Remote working directory
            </span>
            <input
              type="text"
              value={remotePath}
              onChange={(event) => setRemotePath(event.target.value)}
              className="w-full rounded border border-border bg-secondary px-base py-2 font-mono text-sm text-high focus:outline-none focus:ring-1 focus:ring-brand"
            />
          </label>

          {connection.repositories.length > 0 && (
            <div className="space-y-2">
              <span className="text-sm font-medium text-normal">
                Detected Git repositories
              </span>
              <div className="max-h-36 space-y-1 overflow-y-auto rounded border border-border bg-panel p-1">
                {connection.repositories.map((repository) => (
                  <button
                    type="button"
                    key={repository}
                    onClick={() => setRemotePath(repository)}
                    className={cn(
                      'block w-full truncate rounded px-base py-2 text-left font-mono text-xs hover:bg-secondary',
                      remotePath === repository
                        ? 'bg-brand/10 text-brand'
                        : 'text-normal'
                    )}
                  >
                    {repository}
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              className={primaryButtonClass}
              onClick={() => launch('shell')}
              disabled={!remotePath.trim()}
            >
              <TerminalWindowIcon className="size-icon-sm" />
              Open shell
            </button>
            <button
              type="button"
              className={buttonClass}
              onClick={() => launch('claude')}
              disabled={
                !remotePath.trim() || !toolInstalled(connection, 'claude')
              }
            >
              <CodeIcon className="size-icon-sm" />
              Claude
            </button>
            <button
              type="button"
              className={buttonClass}
              onClick={() => launch('codex')}
              disabled={
                !remotePath.trim() || !toolInstalled(connection, 'codex')
              }
            >
              <CodeIcon className="size-icon-sm" />
              Codex
            </button>
          </div>
        </section>
      )}

      {terminal && (
        <SshTerminal
          key={terminal.key}
          alias={terminal.alias}
          path={terminal.path}
          tool={terminal.tool}
          onClose={() => setTerminal(null)}
        />
      )}
    </div>
  );
}
