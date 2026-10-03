import { describe, expect, test } from 'claude-code/testing'

import { agentsTail, agentWindow, bar, cells, compact, inputTokens, reached, recordStep, statusLine } from '../meter'

const WINDOW = 200_000

describe('meter', () => {
  test('formats tokens compactly', () => {
    expect(compact(950)).toBe('950')
    expect(compact(84_400)).toBe('84k')
    expect(compact(1_000_000)).toBe('1M')
    expect(compact(1_240_000)).toBe('1.2M')
  })

  test('draws a bar clamped to its width', () => {
    expect(bar(42, 10)).toBe('████░░░░░░')
    expect(bar(130, 4)).toBe('████')
  })

  test('splits cells so they add up to the width', () => {
    const split = cells([30, 30, 40], 100, 7)
    expect(split.reduce((a, b) => a + b, 0)).toBe(7)
    expect(cells([150], 100, 10)).toEqual([10])
  })

  test('knows the highest threshold reached', () => {
    expect(reached(null)).toBe(0)
    expect(reached(69)).toBe(0)
    expect(reached(70)).toBe(70)
    expect(reached(91)).toBe(85)
  })

  test('counts every input token a request was answered over', () => {
    expect(
      inputTokens({ input_tokens: 10, output_tokens: 99, cache_read_input_tokens: 1_000, cache_creation_input_tokens: 200 }),
    ).toBe(1_210)
  })

  test('assumes a 200k subagent window until an agent goes past it', () => {
    expect(agentWindow(150_000)).toBe(200_000)
    expect(agentWindow(250_000)).toBe(1_000_000)
  })

  test('folds subagent requests, keeping the peak and the most recent twenty', () => {
    let agents = recordStep({}, 'a', 50_000, 'm', 1)
    agents = recordStep(agents, 'a', 30_000, 'm', 2)
    expect(agents['a']).toEqual({ tokens: 30_000, peak: 50_000, steps: 2, model: 'm', at: 2, type: 'agent', description: '' })

    for (let i = 0; i < 25; i++) agents = recordStep(agents, `x${i}`, 1, 'm', 10 + i)
    expect(Object.keys(agents).length).toBe(20)
    expect(agents['a']).toBeUndefined()
  })

  test('tails the status line with the running subagents', () => {
    const one = { tokens: 120_000, peak: 120_000, steps: 3, model: 'm', at: 0, type: 't', description: 'd' }
    expect(agentsTail([])).toBe('')
    expect(agentsTail([one])).toBe(' · 1 agent (max 120k)')
    expect(agentsTail([one, { ...one, tokens: 40_000 }])).toBe(' · 2 agents (max 120k)')
  })

  test('says when there is no reading yet', () => {
    expect(statusLine({ tokens: null, percent: null, window: WINDOW })).toBe('ctx ░░░░░░░░░░ –/200k')
  })
})

