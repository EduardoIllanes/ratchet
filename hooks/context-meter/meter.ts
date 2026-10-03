import type { ModelUsage, SessionContextUsage } from 'claude-code'

import type { AgentReading, Reading } from './types'

export const THRESHOLDS = [70, 85]
const AGENTS_KEPT = 20

/** What a request was answered over: uncached, cache-written and cache-read input together. */
export function inputTokens(usage: ModelUsage): number {
  return usage.input_tokens + usage.cache_read_input_tokens + usage.cache_creation_input_tokens
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
  const kept = Object.entries(next)
    .sort(([, a], [, b]) => b.at - a.at)
    .slice(0, AGENTS_KEPT)

  return Object.fromEntries(kept)
}

/** The status line's tail for the running subagents: how many, and the fullest. */
export function agentsTail(running: readonly AgentReading[]): string {
  if (running.length === 0) return ''
  const fullest = Math.max(...running.map(agent => agent.tokens))

  return ` · ${running.length} agent${running.length === 1 ? '' : 's'} (max ${compact(fullest)})`
}

export function compact(tokens: number): string {
  if (tokens >= 1_000_000) {
    const millions = tokens / 1_000_000
    return `${Number.isInteger(millions) ? millions : millions.toFixed(1)}M`
  }
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}k`

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
