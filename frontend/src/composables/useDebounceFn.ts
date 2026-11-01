import { onScopeDispose } from 'vue'

export function useDebounceFn<Args extends unknown[]>(fn: (...args: Args) => void, delay = 300) {
  let timer: ReturnType<typeof setTimeout> | undefined

  function cancel() {
    if (timer) clearTimeout(timer)
    timer = undefined
  }

  function run(...args: Args) {
    cancel()
    timer = setTimeout(() => {
      fn(...args)
    }, delay)
  }

  onScopeDispose(cancel)

  return { run, cancel }
}
