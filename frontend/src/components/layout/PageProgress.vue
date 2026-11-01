<script setup lang="ts">
import { computed } from 'vue'

import { usePageProgress } from '@/composables/usePageProgress'

const { active, value } = usePageProgress()

const visible = computed(() => active.value || value.value > 0)
</script>

<template>
  <Transition name="progress">
    <div
      v-if="visible"
      class="pointer-events-none fixed inset-x-0 top-0 z-[90] h-[2px]"
      role="progressbar"
      aria-label="页面加载进度"
      :aria-valuenow="Math.round(value)"
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <div
        class="h-full bg-accent transition-[width] duration-150 ease-out"
        :style="{ width: `${value}%` }"
      />
    </div>
  </Transition>
</template>

<style scoped>
.progress-enter-active,
.progress-leave-active {
  transition: opacity 0.18s ease;
}

.progress-enter-from,
.progress-leave-to {
  opacity: 0;
}
</style>
