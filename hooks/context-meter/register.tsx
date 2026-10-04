import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, SessionContextUsage } from 'claude-code'

import type { AgentReading, TaskCost } from './types'
import {
  agentWindow,
  agentsTail,
  bar,
  categoryColors,
  CELESTIAL,
  cappedPath,
  cells,
  compact,
  fillColor,
  inputTokens,
  lastDelta,
  pushHistory,
  reached,
  recordStep,
  shortType,
  sparkline,
  statusLine,
  taskCost,
  toReading,
  topConsumers,
  turnsToCompact,
} from './meter'

const PANE = 'context-meter'

const reading = atom({ plugin: 'ratchet', key: 'contextReading' } as const, null)
const alerted = atom({ plugin: 'ratchet', key: 'contextAlerted' } as const, 0)
const agents = atom({ plugin: 'ratchet', key: 'contextAgents' } as const, {})
const history = atom({ plugin: 'ratchet', key: 'contextHistory' } as const, [])
const task = atom({ plugin: 'ratchet', key: 'contextTask' } as const, null)
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

/** The plugin's own binary: `ratchet.exe` where it exists as a file (Windows), else `ratchet`. */
async function binary($: EngineInterface): Promise<string> {
  const exe = `${$.plugin.root}/bin/ratchet.exe`
  try {
    if ((await $.fs.stat(exe)).kind === 'file') return exe
  } catch {
    // A stat that rejects counts as absent.
  }
  return `${$.plugin.root}/bin/ratchet`
}

/** A command's stdout, or the whole output from the file when ratchet capped it. */
async function fullOutput($: EngineInterface, stdout: string): Promise<string> {
  const path = cappedPath(stdout)

  return path === null ? stdout : await $.fs.read(path)
}

/**
 * Reads what the held task has cost so far; off the hot path, on session start and /ctx only:
 * a report past ratchet's output cap leaves a file under ~/.ratchet/out on every read.
 */
async function refreshTask($: EngineInterface): Promise<void> {
  try {
    const ratchet = await binary($)
    const session = await $.session.id()
    const listed = await $.process.run([ratchet, 'task', 'list', '--mine', '--json', '--session', session], {
      timeoutMs: 15_000,
    })
    if (listed.exitCode !== 0) return
    const held = (JSON.parse(await fullOutput($, listed.stdout)) as { id: string; title: string; status: string }[]).find(
      one => one.status === 'in_progress',
    )
    if (held === undefined) {
      await update($, task, () => null)
      return
    }
    const used = await $.process.run([ratchet, 'usage', held.id, '--json'], { timeoutMs: 30_000 })
    if (used.exitCode !== 0) return
    const cost = taskCost(JSON.parse(await fullOutput($, used.stdout)), held.id)
    // A report with nothing for the task keeps the last figures.
    if (cost !== null) await update($, task, () => ({ id: held.id, title: held.title, ...cost }))
  } catch {
    // The meter never fails the session; the pane keeps the last figures.
  }
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
  const turns = await read($, history)
  // A failing agent list leaves the line without its tail.
  const tail = await running($).then(
    list => agentsTail(list, now.window),
    () => '',
  )
  $.ui.status(statusLine(now) + lastDelta(turns) + tail)
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
    try {
      const opted = await optedIn($, e.cwd)
      await update($, metered, () => opted)
      if (opted) {
        // Without the command the line is still worth pinning.
        await $.command
          .register({
            name: 'ctx',
            description: 'Show the context window broken down by category, live, in a pane',
            immediate: true,
          })
          .catch(() => undefined)
        void refreshTask($)
        await show($, (await $.session.usage()).context)
      }
    } catch {
      // The meter never fails the session.
    }

    return next(e)
  })

  on('command.run', { command: 'ctx' }, async $ => {
    await $.ui.open({ id: PANE, title: 'Context' })
    void refreshTask($)

    return { text: 'Context pane opened.' }
  })

  on('session.measure', async ($, e, next) => {
    if (e.changed.includes('context') && (await read($, metered))) {
      try {
        // One point per turn: the trend and the status line's move read these.
        const tokens = e.context.tokens
        if (tokens !== undefined) await update($, history, all => pushHistory(all, tokens))
      } catch {
        // A failed history must not keep the line from refreshing.
      }
      try {
        await show($, e.context)
      } catch {
        // The meter never fails the session.
      }
    }

    return next(e)
  })

  // A /clear empties the window: what was tracked for the old one goes with it.
  on('session.end', async ($, e, next) => {
    if (e.reason === 'clear') {
      try {
        await update($, agents, () => ({}))
        await update($, history, () => [])
        await update($, task, () => null)
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
        const who = await $.agent.list().then(
          list => list.find(agent => agent.id === id),
          () => undefined,
        )
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
    try {
      // Read to subscribe: each new reading redraws the pane.
      await read($, reading)
      const readings = await read($, agents)
      const turns = await read($, history)
      const held = await read($, task)
      const said = await $.session.messages({ as: 'api' }).catch(() => [])
      const consumers = topConsumers(Array.isArray(said) ? said : [], 5)
      const context = await $.session.usage({ breakdown: 'summary' }).then(
        usage => usage.context,
        () => undefined,
      )
      const breakdown = context?.breakdown

      if (context === undefined || breakdown === undefined) {
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
      const colors = categoryColors(rows.map(row => row.kind))
      // The agent list drops an agent once it is done; the readings keep who it was.
      const listed = await $.agent.list().catch(() => [])
      const isRunning = new Set(listed.filter(agent => agent.status === 'running').map(agent => agent.id))
      const roleCells = held === null ? [] : cells(held.roles.map(role => role.tokens), held.total, width)
      const recent = Object.entries(readings).sort(([, a], [, b]) => b.at - a.at)

      return (
        <Box flexDirection="column">
          <Text bold>
            <Text color={fillColor(breakdown.percentage)}>{breakdown.percentage}%</Text> · {compact(breakdown.totalTokens)} /{' '}
            {compact(breakdown.rawMaxTokens)}
            <Text dimColor> {breakdown.model}</Text>
          </Text>
          <Box flexDirection="row">
            {rows.map((row, i) => (
              <Text color={colors[i]} dimColor={row.kind === 'free'}>
                {(row.kind === 'used' ? '█' : row.kind === 'buffer' ? '▒' : '░').repeat(widths[i] ?? 0)}
              </Text>
            ))}
          </Box>
          {rows.map((row, i) => (
            <Box flexDirection="row">
              <Text color={colors[i]}>{row.kind === 'used' ? '■ ' : '□ '}</Text>
              <Text dimColor={row.kind !== 'used'}>
                {row.name.padEnd(nameWidth)} {compact(row.tokens).padStart(5)}{' '}
                {((row.tokens / breakdown.rawMaxTokens) * 100).toFixed(1).padStart(5)}%
              </Text>
            </Box>
          ))}
          {breakdown.isAutoCompactEnabled && breakdown.autoCompactThreshold !== undefined && (
            <Text dimColor>auto-compact at {compact(breakdown.autoCompactThreshold)}</Text>
          )}
          {turns.length > 1 && (
            <Text bold color={CELESTIAL.accent}>
              Trend
            </Text>
          )}
          {turns.length > 1 && (
            <Text>
              <Text color={fillColor(breakdown.percentage)}>{sparkline(turns, context.window, width - lastDelta(turns).length)}</Text>
              {lastDelta(turns)}
            </Text>
          )}
          {turns.length > 1 && breakdown.autoCompactThreshold !== undefined && (
            <Text dimColor>
              {(() => {
                const left = turnsToCompact(turns, breakdown.autoCompactThreshold)
                if (left === null) return 'not growing'
                if (left === 0) return 'at the auto-compact threshold'
                return `~${left} turn${left === 1 ? '' : 's'} to auto-compact at this pace`
              })()}
            </Text>
          )}
          {consumers.length > 0 && (
            <Text bold color={CELESTIAL.accent}>
              Top consumers
            </Text>
          )}
          {consumers.map((one, i) => (
            <Box flexDirection="row">
              <Text color={CELESTIAL.series[i % CELESTIAL.series.length]}>{one.tool.padEnd(6)} </Text>
              <Text wrap="truncate-end">
                {compact(one.tokens).padStart(5)} {one.label}{' '}
              </Text>
            </Box>
          ))}
          {consumers.some(one => one.tool === 'Read' && one.tokens >= 5_000) && (
            <Text dimColor>→ reads this size can go to ratchet:reader</Text>
          )}
          {held !== null && held.total > 0 && (
            <Text bold color={CELESTIAL.accent}>
              {held.id} <Text dimColor>· {compact(held.total)} tokens so far</Text>
            </Text>
          )}
          {held !== null && held.total > 0 && (
            <Box flexDirection="row">
              {roleCells.map((count, i) => (
                <Text color={CELESTIAL.series[i % CELESTIAL.series.length]}>{'█'.repeat(count)}</Text>
              ))}
            </Box>
          )}
          {held !== null && held.total > 0 && (
            <Text wrap="truncate-end">
              {held.roles.slice(0, 4).map((role, i) => (
                <Text color={CELESTIAL.series[i % CELESTIAL.series.length]}>
                  {i > 0 ? ' · ' : ''}
                  {shortType(role.role)} {Math.round((role.tokens / held.total) * 100)}%
                </Text>
              ))}
            </Text>
          )}
          {recent.length > 0 && (
            <Text bold color={CELESTIAL.accent}>
              Subagents
            </Text>
          )}
          {recent.map(([id, one]) => {
            const isLive = isRunning.has(id)
            const window = agentWindow(one.peak, context.window)
            const percent = Math.round((one.tokens / window) * 100)
            const peak = one.peak > one.tokens ? ` · peak ${compact(one.peak)}` : ''
            const numbers = ` ${percent}% · ${compact(one.tokens)}/${compact(window)} · ${one.steps} req${peak}`
            const room = Math.max(6, Math.min(20, e.props.bodyColumns - 2 - numbers.length))
            return (
              <Box flexDirection="column">
                <Text dimColor={!isLive} wrap="truncate-end">
                  {isLive ? '● ' : '○ '}
                  <Text bold>{one.type}</Text>
                  {one.description ? `  ${one.description}` : ''}
                </Text>
                <Box flexDirection="row">
                  <Text>{'  '}</Text>
                  <Text color={fillColor(percent)} dimColor={!isLive}>
                    {bar(percent, room)}
                  </Text>
                  <Text dimColor={!isLive}>{numbers}</Text>
                </Box>
              </Box>
            )
          })}
        </Box>
      )
    } catch {
      // Whatever fails while drawing, the pane still says something.
      return <Text dimColor>No breakdown yet: it arrives with the first response.</Text>
    }
  })
}
