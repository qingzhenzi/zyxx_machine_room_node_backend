<script setup lang="ts">
import { computed } from 'vue'

import { courseColor, type BoardBlock } from './boardTypes'

const props = withDefaults(
  defineProps<{
    block: BoardBlock
    compact?: boolean
    showRoom?: boolean
    selected?: boolean
    dragging?: boolean
  }>(),
  { compact: false, showRoom: true, selected: false, dragging: false },
)

const color = computed(() => courseColor(props.block.course_name))
const hasRoomConflict = computed(() =>
  props.block.conflicts.some((conflict) => conflict.kind === 'room'),
)
const hasTeacherConflict = computed(() =>
  props.block.conflicts.some((conflict) => conflict.kind === 'teacher'),
)
const conflictLabel = computed(() => {
  if (hasRoomConflict.value) return '房间冲突'
  if (hasTeacherConflict.value) return '教师冲突'
  return ''
})
</script>

<template>
  <span
    class="board-chip"
    :class="{
      'is-compact': compact,
      'is-selected': selected,
      'is-dragging': dragging,
      'has-error': hasRoomConflict,
      'has-warning': !hasRoomConflict && hasTeacherConflict,
    }"
    :style="{ '--chip-fg': color.fg, '--chip-bg': color.bg }"
    :title="`${block.course_name} · ${block.room_code} · ${block.start_time}–${block.end_time}`"
  >
    <span class="chip-main">
      <span class="chip-course">{{ block.course_name }}</span>
      <span class="chip-badges">
        <span v-if="block.is_override" class="chip-badge is-ov">调</span>
        <span v-if="block.source === 'exception'" class="chip-badge is-ex">单日</span>
      </span>
    </span>
    <span class="chip-meta">
      <span v-if="showRoom" class="chip-room">{{ block.room_code }}</span>
      <span v-if="block.teacher" class="chip-teacher">{{ block.teacher }}</span>
    </span>
    <span v-if="conflictLabel" class="chip-conflict" :class="{ 'is-error': hasRoomConflict }">
      {{ conflictLabel }}
    </span>
  </span>
</template>

<style scoped>
.board-chip {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  padding: 3px 6px;
  border: 1px solid var(--chip-fg);
  border-left-width: 3px;
  border-radius: 5px;
  background: var(--chip-bg);
  color: var(--color-ink);
  font-size: 11.5px;
  line-height: 1.25;
  text-align: left;
  overflow: hidden;
}

.board-chip.is-compact {
  padding: 2px 5px;
  font-size: 10.5px;
}

.board-chip.is-selected {
  outline: 2px solid var(--color-accent);
  outline-offset: 1px;
}

.board-chip.is-dragging {
  opacity: 0.45;
}

.board-chip.has-error {
  border-color: #dc2626;
  box-shadow: 0 0 0 1px #dc2626 inset;
  background-image: repeating-linear-gradient(
    -45deg,
    rgba(220, 38, 38, 0.08) 0,
    rgba(220, 38, 38, 0.08) 4px,
    transparent 4px,
    transparent 8px
  );
}

.board-chip.has-warning {
  border-color: #d97706;
  box-shadow: 0 0 0 1px #d97706 inset;
  background-image: repeating-linear-gradient(
    -45deg,
    rgba(217, 119, 6, 0.08) 0,
    rgba(217, 119, 6, 0.08) 4px,
    transparent 4px,
    transparent 8px
  );
}

.chip-main {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}

.chip-course {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
  color: var(--chip-fg);
}

.chip-badges {
  display: inline-flex;
  flex-shrink: 0;
  gap: 2px;
}

.chip-badge {
  display: inline-flex;
  align-items: center;
  padding: 0 3px;
  border-radius: 3px;
  font-size: 9px;
  line-height: 13px;
  font-weight: 600;
}

.chip-badge.is-ov {
  background: #fff7e6;
  color: #b45309;
  border: 1px solid #f3c98b;
}

.chip-badge.is-ex {
  background: #eef2ff;
  color: #3a5bd9;
  border: 1px solid #c9d4fb;
}

.chip-meta {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  overflow: hidden;
  color: var(--color-muted);
  font-size: 10px;
  line-height: 1.2;
}

.chip-room,
.chip-teacher {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip-room {
  flex-shrink: 0;
  font-family: var(--font-mono);
}

.chip-teacher {
  min-width: 0;
}

.chip-conflict {
  align-self: flex-start;
  margin-top: 1px;
  padding: 0 3px;
  border-radius: 3px;
  background: #fef3c7;
  color: #92400e;
  font-size: 9px;
  line-height: 13px;
  font-weight: 600;
}

.chip-conflict.is-error {
  background: #fee2e2;
  color: #b91c1c;
}
</style>
