<script setup lang="ts">
withDefaults(
  defineProps<{
    title?: string
    description?: string
    bodyClass?: string
  }>(),
  { title: '', description: '', bodyClass: '' },
)
</script>

<template>
  <section class="rounded-xl border border-line bg-surface">
    <header v-if="title || $slots.header" class="border-b border-line px-5 py-4">
      <slot name="header">
        <h2 class="text-[13.5px] font-semibold text-ink">{{ title }}</h2>
        <p v-if="description" class="mt-0.5 text-[11px] text-muted">{{ description }}</p>
      </slot>
    </header>

    <div :class="bodyClass">
      <slot />
    </div>

    <footer v-if="$slots.footer" class="border-t border-line px-5 py-2.5">
      <slot name="footer" />
    </footer>
  </section>
</template>
