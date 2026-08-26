import { describe, expect, it } from 'vitest'
import { countStages, stageKey, stageLabel } from './Stages'

describe('folding a stage', () => {
  it('trims and lowercases, so two spellings are one stage', () => {
    expect(stageKey(' Drafted ')).toBe('drafted')
    expect(stageKey('DRAFTED')).toBe('drafted')
    expect(stageKey('with-Beta-Readers')).toBe('with-beta-readers')
  })

  /**
   * ASCII case only, which is what SQLite's `lower()` does, so `?stage=` on the
   * server and the grouping here cannot disagree about a stage somebody wrote
   * with an accent in it. `toLowerCase()` would fold the letters below and the
   * backend would leave them, which is two answers to one question.
   */
  it('leaves non-ASCII letters alone, exactly as the backend does', () => {
    expect(stageKey('ÉTAT')).toBe('État')
    expect(stageKey('İLK')).toBe('İlk')
    // And the fold is still a fold: two ASCII spellings of one stage meet.
    expect(stageKey('ÉTAT')).toBe(stageKey('État'))
  })

  /** `todo` reads as a filename; only the stages we know get relabelled. */
  it('relabels only what it knows', () => {
    expect(stageLabel('todo')).toBe('to do')
    expect(stageLabel('TODO')).toBe('to do')
    expect(stageLabel('drafted')).toBe('drafted')
    expect(stageLabel('with-beta-readers')).toBe('with-beta-readers')
  })
})

describe('counting stages', () => {
  it('folds spellings together and shows the canonical one', () => {
    expect(countStages([{ stage: 'Drafted' }, { stage: 'drafted' }])).toEqual([
      { stage: 'drafted', count: 2 },
    ])
  })

  /**
   * Known stages first, in the order a chapter passes through them, so the
   * summary reads left to right as the work moving. Anything else follows, in
   * the order it was met, because an unknown stage has no place in a lifecycle
   * this dashboard invented.
   */
  it('orders the known stages by lifecycle and keeps the rest behind them', () => {
    const counted = countStages([
      { stage: 'final' },
      { stage: 'with-beta-readers' },
      { stage: 'todo' },
      { stage: 'revised' },
      { stage: 'awaiting-a-title' },
      { stage: 'drafted' },
    ])

    expect(counted.map((entry) => entry.stage)).toEqual([
      'todo',
      'drafted',
      'revised',
      'final',
      'with-beta-readers',
      'awaiting-a-title',
    ])
  })

  /**
   * An unstaged section is not a bucket. Counting it would be inventing the
   * opinion this whole feature refuses to derive: a chapter is at a stage when
   * the author says so.
   */
  it('does not count a section that says nothing', () => {
    expect(countStages([{}, { stage: null }, { stage: '' }, { stage: '  ' }])).toEqual([])
  })

  /** An unknown stage keeps whatever spelling it arrived in. */
  it('keeps an unknown stage as it was written', () => {
    expect(countStages([{ stage: ' With-Beta-Readers ' }])).toEqual([
      { stage: 'With-Beta-Readers', count: 1 },
    ])
  })
})
