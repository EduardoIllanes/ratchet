import type { ModelUsage, SessionContextUsage } from 'claude-code'

import type { AgentReading, Reading } from './types'

export const THRESHOLDS = [70, 85]
const AGENTS_KEPT = 20

/** What a request was answered over: uncached, cache-written and cache-read input together. */
export function inputTokens(usage: ModelUsage): number {
  return usage.input_tokens + usage.cache_read_input_tokens + usage.cache_creation_input_tokens
}

/**
 * The window a subagent runs in. The engine reports none per agent, so take the
 * session model's window, or the agent's peak when it went past that.
 */
export function agentWindow(peak: number, sessionWindow: number): number {
  return Math.max(peak, sessionWindow)
}

/** The Celestial theme's palette (a darker Horizon), by role. */
export const CELESTIAL = {
  calm: '#29D398',
  warm: '#FAB795',
  hot: '#E95678',
  accent: '#B877DB',
  muted: '#6C6F93',
  series: ['#26BBD9', '#B877DB', '#F09483', '#EE64AC', '#59E1E3', '#FAB795', '#29D398'],
} as const

/** The colour of a fill: calm, then warm at the first threshold, hot at the second. */
export function fillColor(percent: number): string {
  if (percent >= THRESHOLDS[1]!) return CELESTIAL.hot
  if (percent >= THRESHOLDS[0]!) return CELESTIAL.warm

  return CELESTIAL.calm
}

/** One colour per breakdown row: the series for what is used, muted for the buffer and free space. */
export function categoryColors(kinds: readonly string[]): string[] {
  let used = 0

  return kinds.map(kind =>
    kind === 'used' ? CELESTIAL.series[used++ % CELESTIAL.series.length]! : CELESTIAL.muted,
  )
}

/** Folds one request of a subagent into the readings, keeping the most recent few. */
export function recordStep(
  agents: Record<string, AgentReading>,
  id: string,
  tokens: number,
  model: string,
  at: number,
  who?: { type: string; description: string },
): Record<string, AgentReading> {
  const before = agents[id]
  const next = {
    ...agents,
    [id]: {
      tokens,
      peak: Math.max(tokens, before?.peak ?? 0),
      steps: (before?.steps ?? 0) + 1,
      model,
      at,
      type: who?.type ?? before?.type ?? 'agent',
      description: who?.description ?? before?.description ?? '',
    },
  }
  // Ties on `at` keep the agent just updated.
  const kept = Object.entries(next)
    .sort(([keyA, a], [keyB, b]) => b.at - a.at || Number(keyB === id) - Number(keyA === id))
    .slice(0, AGENTS_KEPT)

  return Object.fromEntries(kept)
}

/** An agent type without its plugin prefix: `ratchet:reader` reads `reader`. */
export function shortType(type: string): string {
  return type.slice(type.indexOf(':') + 1)
}

const TAIL_AGENTS = 3

/** The status line's tail: a small bar per running subagent, the fullest first, `+N` past three. */
export function agentsTail(running: readonly AgentReading[], sessionWindow: number): string {
  if (running.length === 0) return ''
  const shown = [...running].sort((a, b) => b.tokens - a.tokens).slice(0, TAIL_AGENTS)
  const parts = shown.map(agent => {
    const percent = Math.round((agent.tokens / agentWindow(agent.peak, sessionWindow)) * 100)
    return `${shortType(agent.type)} ${bar(percent, 5)} ${percent}%`
  })
  const more = running.length > shown.length ? ` +${running.length - shown.length}` : ''

  return ` │ ${parts.join(' · ')}${more}`
}

export function compact(tokens: number): string {
  if (tokens >= 1_000) {
    const thousands = Math.round(tokens / 1_000)
    // 999600 rounds to 1000k: that is a million.
    if (thousands < 1_000) return `${thousands}k`
    return `${(tokens / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
  }

  return String(tokens)
}

export function bar(percent: number, width: number): string {
  const filled = Math.max(0, Math.min(width, Math.round((percent / 100) * width)))

  return '█'.repeat(filled) + '░'.repeat(width - filled)
}

export function toReading(context: SessionContextUsage): Reading {
  return {
    tokens: context.tokens ?? null,
    window: context.window,
    percent: context.percent ?? null,
  }
}

export function statusLine(reading: Reading): string {
  if (reading.tokens === null || reading.percent === null) {
    return `ctx ${bar(0, 10)} –/${compact(reading.window)}`
  }

  return `ctx ${bar(reading.percent, 10)} ${reading.percent}% · ${compact(reading.tokens)}/${compact(reading.window)}`
}

/** The highest threshold the fill has reached; 0 below all of them or with no reading. */
export function reached(percent: number | null): number {
  return THRESHOLDS.filter(t => (percent ?? 0) >= t).at(-1) ?? 0
}

/** Splits `width` cells among `parts` in proportion to `total`, rounding cumulatively so the cells add up. */
export function cells(parts: readonly number[], total: number, width: number): number[] {
  let sum = 0
  let start = 0

  return parts.map(part => {
    sum += part
    const end = Math.min(width, Math.round((sum / total) * width))
    const count = Math.max(0, end - start)
    start = Math.max(start, end)
    return count
  })
}

const HISTORY_KEPT = 60
const LEVELS = '▁▂▃▄▅▆▇█'

/** Appends one turn's main-window tokens, keeping the most recent sixty. */
export function pushHistory(history: readonly number[], tokens: number): number[] {
  return [...history, tokens].slice(-HISTORY_KEPT)
}

/** The last turn's move, `+12k` or `-30k`; empty with fewer than two turns or no move. */
export function lastDelta(history: readonly number[]): string {
  const last = history.at(-1)
  const before = history.at(-2)
  if (last === undefined || before === undefined || last === before) return ''

  return last > before ? ` +${compact(last - before)}` : ` -${compact(before - last)}`
}

/** One block per turn, its height the window's fill at that turn. */
export function sparkline(history: readonly number[], window: number, width: number): string {
  return history
    .slice(-width)
    .map(tokens => LEVELS[Math.min(7, Math.floor((tokens / window) * 8))])
    .join('')
}

/**
 * Turns left before auto-compaction at the pace of the last few turns since the
 * window last shrank (a compaction or /clear); null when it is not growing.
 */
export function turnsToCompact(history: readonly number[], threshold: number): number | null {
  const growth: number[] = []
  for (let i = history.length - 1; i > 0 && growth.length < 5; i--) {
    const step = history[i]! - history[i - 1]!
    if (step < 0) break
    growth.push(step)
  }
  const pace = growth.reduce((a, b) => a + b, 0) / Math.max(1, growth.length)
  const now = history.at(-1) ?? 0
  if (now >= threshold) return 0
  if (pace <= 0) return null

  return Math.ceil((threshold - now) / pace)
}

/** A rough token count of a tool result as the model read it: four characters a token. */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4)
}

/** What a tool call was about, in a few words: the file, the command, the pattern. */
export function toolLabel(tool: string, input: Record<string, unknown>): string {
  const text = (key: string) => (typeof input[key] === 'string' ? (input[key] as string) : '')
  const path = text('file_path') || text('notebook_path') || text('path')
  if (path !== '') return path.split(/[\\/]/).slice(-3).join('/')
  const command = text('command').split('\n')[0] ?? ''
  if (command !== '') return command.length > 48 ? `${command.slice(0, 47)}…` : command

  return text('pattern') || text('url') || text('description') || text('subagent_type') || text('query')
}

export type Consumer = { tool: string; label: string; tokens: number }

type ApiBlock = { type: string; [field: string]: unknown }

/** A tool result's text: its content when a string, else its text blocks joined. */
function resultText(content: unknown): string {
  if (typeof content === 'string') return content
  if (!Array.isArray(content)) return ''

  return (content as ApiBlock[])
    .filter(block => block.type === 'text' && typeof block.text === 'string')
    .map(block => block.text as string)
    .join('')
}

/** The heaviest tool results among Messages API messages, largest first. */
export function topConsumers(messages: readonly { role: string; content: readonly ApiBlock[] }[], count: number): Consumer[] {
  const blocks = messages.flatMap(message => (Array.isArray(message.content) ? message.content : []))
  const uses = new Map<string, { name: string; input: Record<string, unknown> }>()
  for (const block of blocks) {
    if (block.type === 'tool_use' && typeof block.id === 'string') {
      uses.set(block.id, { name: String(block.name), input: (block.input ?? {}) as Record<string, unknown> })
    }
  }

  return blocks
    .flatMap(block => {
      const use = block.type === 'tool_result' ? uses.get(String(block.tool_use_id)) : undefined
      if (use === undefined) return []
      return [{ tool: use.name, label: toolLabel(use.name, use.input), tokens: estimateTokens(resultText(block.content)) }]
    })
    .sort((a, b) => b.tokens - a.tokens)
    .slice(0, count)
}

/** The path a capped output names: its last non-empty line reads `… (<n> lines in <path>)`. */
export function cappedPath(stdout: string): string | null {
  const lines = stdout.trimEnd().split('\n')
  const match = /^… \(\d+ lines in (.+)\)$/.exec(lines.at(-1) ?? '')

  return match?.[1] ?? null
}

/** Sums `ratchet usage <id> --json`'s buckets by role, input to output (thinking is inside output). */
export function taskCost(json: unknown, id: string): { total: number; roles: { role: string; tokens: number }[] } | null {
  const tasks = (json as { tasks?: { id?: string; buckets?: { role?: string; tokens?: Record<string, number> }[] }[] })?.tasks
  const task = tasks?.find(one => one.id === id)
  if (task === undefined) return null
  const byRole = new Map<string, number>()
  for (const bucket of task.buckets ?? []) {
    const t = bucket.tokens ?? {}
    const sum = (t['input'] ?? 0) + (t['cache_write'] ?? 0) + (t['cache_read'] ?? 0) + (t['output'] ?? 0)
    const role = bucket.role ?? 'unknown'
    byRole.set(role, (byRole.get(role) ?? 0) + sum)
  }
  const roles = [...byRole].map(([role, tokens]) => ({ role, tokens })).sort((a, b) => b.tokens - a.tokens)

  return { total: roles.reduce((a, r) => a + r.tokens, 0), roles }
}
