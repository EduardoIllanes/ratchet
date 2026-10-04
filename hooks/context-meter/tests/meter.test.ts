import { describe, expect, test } from 'claude-code/testing'

import { agentsTail, bar, cappedPath, categoryColors, fillColor, lastDelta, pushHistory, shortType, sparkline, taskCost, topConsumers, turnsToCompact, cells, compact, inputTokens, reached, recordStep, statusLine } from '../meter'

const WINDOW = 200_000

describe('meter', () => {
  test('formats tokens compactly', () => {
    expect(compact(950)).toBe('950')
    expect(compact(84_400)).toBe('84k')
    expect(compact(1_000_000)).toBe('1M')
    expect(compact(1_240_000)).toBe('1.2M')
    expect(compact(999_600)).toBe('1M')
    expect(compact(1_040_000)).toBe('1M')
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

  test('folds subagent requests, keeping the peak and the most recent twenty', () => {
    let agents = recordStep({}, 'a', 50_000, 'm', 1)
    agents = recordStep(agents, 'a', 30_000, 'm', 2)
    expect(agents['a']).toEqual({ tokens: 30_000, peak: 50_000, steps: 2, model: 'm', at: 2, type: 'agent', description: '' })

    for (let i = 0; i < 25; i++) agents = recordStep(agents, `x${i}`, 1, 'm', 10 + i)
    expect(Object.keys(agents).length).toBe(20)
    expect(agents['a']).toBeUndefined()
  })

  test('keeps the agent just updated when others tie on time', () => {
    let agents: ReturnType<typeof recordStep> = {}
    for (let i = 0; i < 21; i++) agents = recordStep(agents, `x${i}`, 1, 'm', 5)
    expect(Object.keys(agents).length).toBe(20)
    expect(agents['x20']).toBeDefined()
  })

  test('tails the status line with the running subagents', () => {
    const one = { tokens: 120_000, peak: 120_000, steps: 3, model: 'm', at: 0, type: 't', description: 'd' }
    const types = ['ratchet:reader', 'Explore', 'ratchet:implementer', 'general-purpose']
    const four = [20_000, 60_000, 160_000, 100_000].map((tokens, i) => ({ ...one, tokens, peak: tokens, type: types[i]! }))
    expect(agentsTail([], WINDOW)).toBe('')
    expect(agentsTail([{ ...one, tokens: 50_000, peak: 50_000, type: 'ratchet:implementer' }], WINDOW)).toBe(
      ' │ implementer █░░░░ 25%',
    )
    expect(agentsTail(four, WINDOW)).toBe(
      ' │ implementer ████░ 80% · general-purpose ███░░ 50% · Explore ██░░░ 30% +1',
    )
  })

  test('says when there is no reading yet', () => {
    expect(statusLine({ tokens: null, percent: null, window: WINDOW })).toBe('ctx ░░░░░░░░░░ –/200k')
  })

  test('strips a plugin prefix and colours by role', () => {
    expect(shortType('ratchet:reader')).toBe('reader')
    expect(shortType('Explore')).toBe('Explore')
    expect(fillColor(69)).toBe('#29D398')
    expect(fillColor(70)).toBe('#FAB795')
    expect(fillColor(85)).toBe('#E95678')
    expect(categoryColors(['used', 'buffer', 'used', 'free'])).toEqual(['#26BBD9', '#6C6F93', '#B877DB', '#6C6F93'])
  })

  test('keeps sixty turns, shows the last move and draws a sparkline', () => {
    expect(pushHistory(Array.from({ length: 60 }, (_, i) => i), 99).length).toBe(60)
    expect(lastDelta([40_000])).toBe('')
    expect(lastDelta([40_000, 52_000])).toBe(' +12k')
    expect(lastDelta([52_000, 22_000])).toBe(' -30k')
    expect(lastDelta([5, 5])).toBe('')
    expect(sparkline([40_000, 50_000, 60_000, 70_000], WINDOW, 40)).toBe('▂▃▃▃')
  })

  test('forecasts the turns to auto-compaction from the pace since the last shrink', () => {
    expect(turnsToCompact([40_000, 50_000, 60_000, 70_000], 167_000)).toBe(10)
    expect(turnsToCompact([100_000, 150_000, 60_000, 70_000], 167_000)).toBe(10)
    expect(turnsToCompact([60_000, 60_000], 167_000)).toBeNull()
    expect(turnsToCompact([100_000, 170_000], 167_000)).toBe(0)
  })

  test('ranks tool results and sums the held task by role', () => {
    const messages = [
      {
        role: 'assistant',
        content: [
          { type: 'tool_use', id: 'g', name: 'Grep', input: { pattern: 'p' } },
          { type: 'tool_use', id: 'r', name: 'Read', input: { file_path: '/a/b/c/d.rs' } },
        ],
      },
      {
        role: 'user',
        content: [
          { type: 'tool_result', tool_use_id: 'g', content: 'x'.repeat(400) },
          { type: 'tool_result', tool_use_id: 'r', content: [{ type: 'text', text: 'x'.repeat(40_000) }, { type: 'image' }] },
        ],
      },
    ]
    const top = topConsumers(messages, 5)
    expect(top).toEqual([
      { tool: 'Read', label: 'b/c/d.rs', tokens: 10_000 },
      { tool: 'Grep', label: 'p', tokens: 100 },
    ])
    const buckets = [
      { role: 'a', tokens: { input: 1, cache_write: 2, cache_read: 3, output: 4 } },
      { role: 'b', tokens: { output: 50 } },
    ]
    expect(taskCost({ tasks: [{ id: 'T-1', buckets }] }, 'T-1')).toEqual({
      total: 60,
      roles: [
        { role: 'b', tokens: 50 },
        { role: 'a', tokens: 10 },
      ],
    })
    expect(taskCost({ tasks: [] }, 'T-1')).toBeNull()
  })

  test('finds the file a capped output names', () => {
    expect(cappedPath('{}\n… (70 lines in /home/u/out/x.txt)\n')).toBe('/home/u/out/x.txt')
    expect(cappedPath('[]')).toBeNull()
  })
})
