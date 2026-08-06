import { describe, expect, it } from 'vitest'
import { formatClock, formatDuration, formatHours, secondsBetween } from './Duration'

describe('formatDuration', () => {
  it('drops seconds above a minute, because a log is skimmed', () => {
    expect(formatDuration(0)).toBe('0s')
    expect(formatDuration(45)).toBe('45s')
    expect(formatDuration(60)).toBe('1m')
    expect(formatDuration(90)).toBe('1m')
    expect(formatDuration(3600)).toBe('1h')
    expect(formatDuration(4470)).toBe('1h 14m')
    expect(formatDuration(36000)).toBe('10h')
  })

  it('never renders a negative duration', () => {
    // The API clamps, but a clock that has gone backwards can still produce
    // one locally, and `-1h 59m` in a table reads as a bug in the totals.
    expect(formatDuration(-500)).toBe('0s')
  })
})

describe('formatClock', () => {
  it('keeps seconds, which is the evidence a timer is running', () => {
    expect(formatClock(0)).toBe('0:00')
    expect(formatClock(9)).toBe('0:09')
    expect(formatClock(90)).toBe('1:30')
    expect(formatClock(3600)).toBe('1:00:00')
    expect(formatClock(4470)).toBe('1:14:30')
  })
})

describe('formatHours', () => {
  it('uses one decimal until the number is big enough not to need it', () => {
    expect(formatHours(0)).toBe('0 h')
    expect(formatHours(1800)).toBe('0.5 h')
    expect(formatHours(4470)).toBe('1.2 h')
    expect(formatHours(3600 * 42)).toBe('42 h')
  })
})

describe('secondsBetween', () => {
  it('counts from an RFC 3339 start', () => {
    const now = new Date('2026-08-06T15:00:00Z')
    expect(secondsBetween('2026-08-06T14:00:00Z', now)).toBe(3600)
  })

  it('is zero rather than negative when the start is in the future', () => {
    const now = new Date('2026-08-06T14:00:00Z')
    expect(secondsBetween('2026-08-06T15:00:00Z', now)).toBe(0)
  })

  it('is zero for a timestamp it cannot read', () => {
    expect(secondsBetween('not a date')).toBe(0)
  })
})
