<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import AppIcon from '@/components/ui/AppIcon.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    title: string
    description?: string
    size?: 'sm' | 'md' | 'lg' | 'xl'
  }>(),
  { size: 'md', description: '' },
)

const emit = defineEmits<{
  close: []
}>()

const panelRef = ref<HTMLElement | null>(null)

const sizeClass = computed(() => {
  switch (props.size) {
    case 'sm':
      return 'max-w-md'
    case 'lg':
      return 'max-w-2xl'
    case 'xl':
      return 'max-w-4xl'
    default:
      return 'max-w-lg'
  }
})

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && props.open) emit('close')
}

watch(
  () => props.open,
  async (open) => {
    if (!open) return
    await nextTick()
    const target =
      panelRef.value?.querySelector<HTMLElement>('[data-autofocus]') ??
      panelRef.value?.querySelector<HTMLElement>('input, select, textarea')
    target?.focus()
  },
)

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div
        v-if="open"
        class="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-ink/30 px-6 py-16 backdrop-blur-[2px]"
        @mousedown.self="emit('close')"
      >
        <div
          ref="panelRef"
          role="dialog"
          aria-modal="true"
          :aria-label="title"
          class="w-full rounded-xl border border-line bg-surface shadow-[0_24px_70px_-24px_rgba(16,24,40,0.4)]"
          :class="sizeClass"
        >
          <header class="flex items-start justify-between gap-4 px-5 pb-4 pt-5">
            <div class="min-w-0">
              <h2 class="text-[15px] font-semibold tracking-[-0.01em] text-ink">
                {{ title }}
              </h2>
              <p v-if="description" class="mt-1 text-[11.5px] leading-5 text-muted">
                {{ description }}
              </p>
            </div>
            <button
              type="button"
              aria-label="关闭"
              class="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-faint transition-colors hover:bg-canvas hover:text-ink"
              @click="emit('close')"
            >
              <AppIcon name="x" :size="15" />
            </button>
          </header>

          <div class="px-5 pb-5">
            <slot />
          </div>

          <footer
            v-if="$slots.footer"
            class="flex items-center justify-end gap-2 border-t border-line px-5 py-3.5"
          >
            <slot name="footer" />
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-enter-active,
.modal-leave-active {
  transition: opacity 0.16s ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}
</style>
