import { computed, onScopeDispose, toValue, watch, type MaybeRefOrGetter } from 'vue'

export function useInterval(callback: () => void, intervalMs: MaybeRefOrGetter<number>) {
  let timer: ReturnType<typeof setInterval> | undefined

  const delay = computed(() => Math.max(1, toValue(intervalMs)))

  function stop() {
    if (timer) clearInterval(timer)
    timer = undefined
  }

  function start() {
    stop()
    timer = setInterval(callback, delay.value)
  }

  watch(delay, start, { immediate: true })
  onScopeDispose(stop)

  return { start, stop }
}
