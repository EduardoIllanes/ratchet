import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, SessionContextUsage } from 'claude-code'

import type { AgentReading } from './types'
import {
  agentWindow,
  agentsTail,
  cells,
  compact,
  inputTokens,
  reached,
  recordStep,
  statusLine,
  toReading,
} from './meter'

const PANE = 'context-meter'

const reading = atom({ plugin: 'ratchet', key: 'contextReading' } as const, null)
const alerted = atom({ plugin: 'ratchet', key: 'contextAlerted' } as const, 0)
const agents = atom({ plugin: 'ratchet', key: 'contextAgents' } as const, {})
// Whether the session opted in to ratchet; decided again on every session.start.
const metered = atom({ plugin: 'ratchet', key: 'contextMetered' } as const, false)

/** The directory and each ancestor, nearest first, splitting on both separators. */
export function ancestors(cwd: string): string[] {
  const parts = cwd.split(/[\\/]/)
  const dirs: string[] = []
  for (let n = parts.length; n >= 1; n--) {
    const dir = parts.slice(0, n).join('/')
    dirs.push(dir === '' ? '/' : dir)
  }
  return dirs
}

async function optedIn($: EngineInterface, cwd: string): Promise<boolean> {
  for (const dir of ancestors(cwd)) {
    try {
      const stat = await $.fs.stat(`${dir === '/' ? '' : dir}/ratchet.toml`)
      if (stat.kind === 'file') return true
    } catch {
      // A stat that rejects counts as absent.
    }
  }
  return false
}

async function running($: EngineInterface): Promise<AgentReading[]> {
  const readings = await read($, agents)
  const listed = await $.agent.list()

  return listed
    .filter(agent => agent.status === 'running')
    .flatMap(agent => readings[agent.id] ?? [])
}

async function pin($: EngineInterface): Promise<void> {
  const now = await read($, reading)
  if (now === null) return
  $.ui.status(statusLine(now) + agentsTail(await running($)))
}

async function show($: EngineInterface, context: SessionContextUsage): Promise<void> {
  const now = toReading(context)

  const level = reached(now.percent)
  const before = await read($, alerted)
  if (level > before) {
    $.ui.toast(`Context at ${now.percent}%: run /compact, or write a handoff with ratchet task handoff and continue in a fresh session`, {
      timeoutMs: 8000,
    })
  }
  // A drop (compaction, /clear) re-arms the thresholds below it.
  if (level !== before) await update($, alerted, () => level)

  await update($, reading, () => now)
  await pin($)
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const opted = await optedIn($, e.cwd)
    await update($, metered, () => opted)
    if (opted) {
      await $.command.register({
        name: 'ctx',
        description: 'Show the context window broken down by category, live, in a pane',
        immediate: true,
      })
      try {
        await show($, (await $.session.usage()).context)
      } catch {
        // The meter never fails the session.
      }
    }

    return next(e)
  })

  on('command.run', { command: 'ctx' }, async $ => {
    await $.ui.open({ id: PANE, title: 'Context' })

    return { text: 'Context pane opened.' }
  })

  on('session.measure', async ($, e, next) => {
    if (e.changed.includes('context') && (await read($, metered))) {
      try {
        await show($, e.context)
      } catch {
        // The meter never fails the session.
      }
    }

    return next(e)
  })

  // Every model request, main or subagent, reports what it was answered over:
  // the main loop's moves the status line within a turn, a subagent's is the
  // only window onto that agent's context.
  on('turn.step', async function* ($, e, next) {
    const result = yield* next(e)
    if (result.usage === null || result.usage === undefined) return result

    try {
      if (!(await read($, metered))) return result
      const tokens = inputTokens(result.usage)
      if (e.agentId === undefined) {
        const { window } = (await $.session.usage()).context
        await show($, { window, tokens, percent: Math.round((tokens / window) * 100) })
      } else {
        const id = e.agentId
        const at = await $.clock.now()
        const who = (await $.agent.list()).find(agent => agent.id === id)
        await update($, agents, all => recordStep(all, id, tokens, result.usage?.model ?? '', at, who))
        await pin($)
      }
    } catch {
      // The meter never fails a model request.
    }

    return result
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    // Read to subscribe: each new reading redraws the pane.
    await read($, reading)
    const readings = await read($, agents)
    const { breakdown } = (await $.session.usage({ breakdown: 'summary' })).context

    if (breakdown === undefined) {
      return <Text dimColor>No breakdown yet: it arrives with the first response.</Text>
    }

    const rows = breakdown.categories.filter(row => row.kind !== 'deferred')
    const width = Math.max(10, e.props.bodyColumns - 2)
    const widths = cells(
      rows.map(row => row.tokens),
      breakdown.rawMaxTokens,
      width,
    )
    const nameWidth = Math.max(...rows.map(row => row.name.length))
    // The agent list drops an agent once it is done; the readings keep who it was.
    const isRunning = new Set((await $.agent.list()).filter(agent => agent.status === 'running').map(agent => agent.id))
    const recent = Object.entries(readings).sort(([, a], [, b]) => b.at - a.at)

    return (
      <Box flexDirection="column">
        <Text bold>
          {breakdown.percentage}% · {compact(breakdown.totalTokens)} / {compact(breakdown.rawMaxTokens)}
          <Text dimColor> {breakdown.model}</Text>
        </Text>
        <Box flexDirection="row">
          {rows.map((row, i) => (
            <Text color={row.color} dimColor={row.kind === 'free'}>
              {(row.kind === 'used' ? '█' : row.kind === 'buffer' ? '▒' : '░').repeat(widths[i] ?? 0)}
            </Text>
          ))}
        </Box>
        {rows.map(row => (
          <Box flexDirection="row">
            <Text color={row.color}>{row.kind === 'used' ? '■ ' : '□ '}</Text>
            <Text dimColor={row.kind !== 'used'}>
              {row.name.padEnd(nameWidth)} {compact(row.tokens).padStart(5)}{' '}
              {((row.tokens / breakdown.rawMaxTokens) * 100).toFixed(1).padStart(5)}%
            </Text>
          </Box>
        ))}
        {breakdown.isAutoCompactEnabled && breakdown.autoCompactThreshold !== undefined && (
          <Text dimColor>auto-compact at {compact(breakdown.autoCompactThreshold)}</Text>
        )}
        {recent.length > 0 && <Text bold>Subagents</Text>}
        {recent.map(([id, one]) => {
          const percent = Math.round((one.tokens / agentWindow(one.peak)) * 100)
          return (
            <Text dimColor={!isRunning.has(id)} wrap="truncate-end">
              {isRunning.has(id) ? '● ' : '○ '}
              {one.type ?? 'agent'} · {compact(one.tokens)} ~{percent}% · peak {compact(one.peak)} · {one.steps} req ·{' '}
              {one.description ?? ''}
            </Text>
          )
        })}
      </Box>
    )
  })
}
