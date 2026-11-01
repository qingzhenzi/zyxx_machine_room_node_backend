<script setup lang="ts">
import { computed } from 'vue'

import type { Permission } from '@/api/schema'
import { groupPermissions, permissionName } from '@/features/roles/permissionGroups'

const props = withDefaults(
  defineProps<{
    modelValue: string[]
    permissions: Permission[]
    disabled?: boolean
  }>(),
  { disabled: false },
)

const emit = defineEmits<{
  'update:modelValue': [value: string[]]
}>()

const groups = computed(() => groupPermissions(props.permissions))
const allCodes = computed(() => props.permissions.map((permission) => permission.code ?? ''))
const selectedCount = computed(
  () => props.modelValue.filter((code) => allCodes.value.includes(code)).length,
)
const allSelected = computed(
  () => allCodes.value.length > 0 && selectedCount.value === allCodes.value.length,
)

function emitSelection(next: string[]) {
  emit('update:modelValue', next)
}

function toggle(code: string) {
  if (!code || props.disabled) return
  if (props.modelValue.includes(code)) {
    emitSelection(props.modelValue.filter((item) => item !== code))
  } else {
    emitSelection([...props.modelValue, code])
  }
}

function selectAll() {
  emitSelection([...allCodes.value])
}

function clearAll() {
  emitSelection([])
}

function groupSelected(codes: string[]): number {
  return codes.filter((code) => props.modelValue.includes(code)).length
}

function setGroup(codes: string[], selected: boolean) {
  if (props.disabled) return
  if (selected) {
    emitSelection(Array.from(new Set([...props.modelValue, ...codes])))
  } else {
    emitSelection(props.modelValue.filter((code) => !codes.includes(code)))
  }
}
</script>

<template>
  <div>
    <div class="mb-2 flex items-center justify-between">
      <p class="text-[11px] text-muted">
        已选 <span class="font-mono text-ink-soft">{{ selectedCount }}</span> /
        {{ permissions.length }} 个权限点
      </p>
      <div v-if="!disabled" class="flex items-center gap-3 text-[11px]">
        <button
          type="button"
          class="link disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="allSelected"
          @click="selectAll"
        >
          全选
        </button>
        <button
          type="button"
          class="text-muted transition-colors hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="selectedCount === 0"
          @click="clearAll"
        >
          清空
        </button>
      </div>
    </div>

    <div
      class="max-h-[356px] divide-y divide-line/70 overflow-y-auto rounded-md border border-line"
    >
      <section v-for="group in groups" :key="group.key" class="px-3 py-2.5">
        <header class="mb-1.5 flex items-center justify-between gap-3">
          <div class="flex items-center gap-2">
            <p class="text-[11.5px] font-medium text-ink">{{ group.label }}</p>
            <span class="font-mono text-[9.5px] text-faint">
              {{ group.permissions.length }}
            </span>
            <span class="text-[9.5px] text-faint">
              已选 {{ groupSelected(group.permissions.map((item) => item.code ?? '')) }}
            </span>
          </div>
          <div v-if="!disabled" class="flex items-center gap-2 text-[10.5px]">
            <button
              type="button"
              class="link"
              @click="
                setGroup(
                  group.permissions.map((item) => item.code ?? ''),
                  true,
                )
              "
            >
              全选
            </button>
            <button
              type="button"
              class="text-faint transition-colors hover:text-ink"
              @click="
                setGroup(
                  group.permissions.map((item) => item.code ?? ''),
                  false,
                )
              "
            >
              清空
            </button>
          </div>
        </header>

        <div class="grid grid-cols-2 gap-x-4 gap-y-1">
          <label
            v-for="permission in group.permissions"
            :key="permission.code"
            class="flex items-start gap-2 rounded px-1 py-1 transition-colors"
            :class="disabled ? 'cursor-default' : 'cursor-pointer hover:bg-canvas/70'"
          >
            <input
              type="checkbox"
              :checked="modelValue.includes(permission.code ?? '')"
              :disabled="disabled"
              class="mt-0.5 h-3.5 w-3.5 shrink-0 rounded border-line text-accent focus:ring-accent/25 disabled:cursor-not-allowed"
              @change="toggle(permission.code ?? '')"
            />
            <span class="min-w-0">
              <span
                class="block truncate text-[11px] text-ink-soft"
                :title="permissionName(permission)"
              >
                {{ permissionName(permission) }}
              </span>
              <span class="block truncate font-mono text-[9.5px] text-faint">
                {{ permission.code }}
              </span>
            </span>
          </label>
        </div>
      </section>
    </div>
  </div>
</template>
