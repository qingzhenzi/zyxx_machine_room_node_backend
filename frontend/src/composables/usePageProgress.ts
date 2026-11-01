import { readonly, ref } from 'vue'

const active = ref(false)
const value = ref(0)

const MIN_RAMP_MS = 520
const COMPLETE_HOLD_MS = 240
let startedAt = 0
let ticker: ReturnType<typeof setInterval> | undefined
let hideTimer: ReturnType<typeof setTimeout> | undefined
let resetTimer: ReturnType<typeof setTimeout> | undefined
let safetyTimer: ReturnType<typeof setTimeout> | undefined
let completeTimer: ReturnType<typeof setTimeout> | undefined

function clearTicker() {
  if (ticker) clearInterval(ticker)
  ticker = undefined
}

function clearTimers() {
  if (hideTimer) clearTimeout(hideTimer)
  if (resetTimer) clearTimeout(resetTimer)
  if (safetyTimer) clearTimeout(safetyTimer)
  if (completeTimer) clearTimeout(completeTimer)
  hideTimer = undefined
  resetTimer = undefined
  safetyTimer = undefined
  completeTimer = undefined
}

export function startPageProgress() {
  clearTimers()
  clearTicker()

  active.value = true
  startedAt = Date.now()
  value.value = 8

  ticker = setInterval(() => {
    value.value = Math.min(92, value.value + (92 - value.value) * 0.16)
  }, 120)

  // 防止路由被取消后进度条永久停留
  safetyTimer = setTimeout(() => {
    finishPageProgress()
  }, 7_000)
}

export function finishPageProgress() {
  if (!active.value || completeTimer) return

  const elapsed = Date.now() - startedAt
  const wait = Math.max(80, MIN_RAMP_MS - elapsed)

  completeTimer = setTimeout(() => {
    completeTimer = undefined
    clearTicker()
    if (safetyTimer) clearTimeout(safetyTimer)
    safetyTimer = undefined
    value.value = 100

    hideTimer = setTimeout(() => {
      active.value = false
      resetTimer = setTimeout(() => {
        value.value = 0
      }, 260)
    }, COMPLETE_HOLD_MS)
  }, wait)
}

export function usePageProgress() {
  return {
    active: readonly(active),
    value: readonly(value),
  }
}
