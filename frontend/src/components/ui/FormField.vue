<script setup lang="ts">
withDefaults(
  defineProps<{
    label: string
    inputId?: string
    hint?: string
    error?: string
    required?: boolean
  }>(),
  { inputId: '', hint: '', error: '', required: false },
)
</script>

<template>
  <div>
    <label
      v-if="label"
      :for="inputId || undefined"
      class="mb-1.5 block text-[11.5px] font-medium text-ink-soft"
    >
      {{ label }}
      <span v-if="required" class="text-rose-500">*</span>
    </label>
    <slot />
    <p v-if="error" class="mt-1.5 text-[11px] text-rose-600">{{ error }}</p>
    <p v-else-if="hint" class="mt-1.5 text-[10.5px] text-faint">{{ hint }}</p>
  </div>
</template>
