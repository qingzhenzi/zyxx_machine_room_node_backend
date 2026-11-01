import { defineStore } from 'pinia'
import { ref } from 'vue'

export type ToastKind = 'success' | 'error' | 'info'

export interface ToastItem {
  id: number
  kind: ToastKind
  title: string
  description?: string
}

export const useToastStore = defineStore('toast', () => {
  const items = ref<ToastItem[]>([])
  const timers = new Map<number, ReturnType<typeof setTimeout>>()
  let sequence = 0

  function dismiss(id: number) {
    const timer = timers.get(id)
    if (timer) clearTimeout(timer)
    timers.delete(id)
    items.value = items.value.filter((item) => item.id !== id)
  }

  function push(kind: ToastKind, title: string, description?: string) {
    const id = ++sequence
    items.value.push({ id, kind, title, description })
    timers.set(
      id,
      setTimeout(() => dismiss(id), 4_500),
    )
    return id
  }

  const success = (title: string, description?: string) => push('success', title, description)
  const error = (title: string, description?: string) => push('error', title, description)
  const info = (title: string, description?: string) => push('info', title, description)

  return { items, push, dismiss, success, error, info }
})
