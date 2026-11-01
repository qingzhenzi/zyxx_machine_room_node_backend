<script setup lang="ts">
withDefaults(
  defineProps<{
    name: string
    size?: number
  }>(),
  { size: 16 },
)
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    class="shrink-0"
  >
    <template v-if="name === 'overview'">
      <rect x="3" y="3" width="7" height="7" rx="1.5" />
      <rect x="14" y="3" width="7" height="7" rx="1.5" />
      <rect x="3" y="14" width="7" height="7" rx="1.5" />
      <rect x="14" y="14" width="7" height="7" rx="1.5" />
    </template>
    <template v-else-if="name === 'projects'">
      <path
        d="M3 7.5A1.5 1.5 0 0 1 4.5 6H9l2 2h8.5A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"
      />
    </template>
    <template v-else-if="name === 'runs'">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 1.8" />
    </template>
    <template v-else-if="name === 'reports'">
      <path d="M4 20V10M10 20V4M16 20v-7M22 20H2" />
    </template>
    <template v-else-if="name === 'server'">
      <rect x="3" y="4" width="18" height="7" rx="1.5" />
      <rect x="3" y="13" width="18" height="7" rx="1.5" />
      <path d="M7 7.5h.01M7 16.5h.01" />
    </template>
    <template v-else-if="name === 'settings'">
      <path d="M4 7h9M19 7h1M4 17h3M13 17h7" />
      <circle cx="16" cy="7" r="2.2" />
      <circle cx="10" cy="17" r="2.2" />
    </template>
    <template v-else-if="name === 'search'">
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4 4" />
    </template>
    <template v-else-if="name === 'plus'">
      <path d="M12 5v14M5 12h14" />
    </template>
    <template v-else-if="name === 'chevron-down'">
      <path d="m6 9 6 6 6-6" />
    </template>
    <template v-else-if="name === 'chevron-right'">
      <path d="m9 6 6 6-6 6" />
    </template>
    <template v-else-if="name === 'bell'">
      <path d="M18 8a6 6 0 1 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9" />
      <path d="M10.5 21a2 2 0 0 0 3 0" />
    </template>
    <template v-else-if="name === 'play'">
      <path d="M8 5.5v13l10.5-6.5z" />
    </template>
    <template v-else-if="name === 'arrow-up-right'">
      <path d="M7 17 17 7M9 7h8v8" />
    </template>
    <template v-else-if="name === 'download'">
      <path d="M12 4v11m0 0 4-4m-4 4-4-4M5 19h14" />
    </template>
    <template v-else-if="name === 'spark'">
      <path
        d="M12 3v3.5M12 17.5V21M3 12h3.5M17.5 12H21M5.6 5.6l2.5 2.5M15.9 15.9l2.5 2.5M18.4 5.6l-2.5 2.5M8.1 15.9l-2.5 2.5"
      />
    </template>
    <template v-else-if="name === 'x'">
      <path d="M6 6l12 12M18 6 6 18" />
    </template>
    <template v-else-if="name === 'check'">
      <path d="m5 12 4.5 4.5L19 7" />
    </template>
    <template v-else-if="name === 'refresh'">
      <path d="M20 11a8 8 0 1 0-2.3 6.4M20 5v6h-6" />
    </template>
    <template v-else-if="name === 'alert'">
      <path d="M12 4 3 20h18z" />
      <path d="M12 9.5v4.5M12 17.5h.01" />
    </template>
    <template v-else-if="name === 'info'">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11v5M12 7.8h.01" />
    </template>
    <template v-else-if="name === 'user'">
      <circle cx="12" cy="8" r="3.5" />
      <path d="M4.5 20a7.5 7.5 0 0 1 15 0" />
    </template>
    <template v-else-if="name === 'lock'">
      <rect x="4.5" y="10.5" width="15" height="9.5" rx="2" />
      <path d="M8 10.5V7.8a4 4 0 0 1 8 0v2.7" />
    </template>
    <template v-else-if="name === 'eye'">
      <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z" />
      <circle cx="12" cy="12" r="3" />
    </template>
    <template v-else-if="name === 'eye-off'">
      <path d="m4 4 16 16" />
      <path
        d="M9.9 5.9A9.8 9.8 0 0 1 12 5.5c6 0 9.5 6.5 9.5 6.5a17.4 17.4 0 0 1-2.4 3.2M6.6 7.7A16.7 16.7 0 0 0 2.5 12S6 18.5 12 18.5c1 0 1.9-.2 2.7-.5"
      />
      <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
    </template>
    <template v-else-if="name === 'shield'">
      <path d="M12 3 5 6v5.5c0 4.4 3 7.6 7 9.5 4-1.9 7-5.1 7-9.5V6z" />
      <path d="m9 12 2 2 4-4" />
    </template>
    <template v-else-if="name === 'arrow-right'">
      <path d="M4 12h16M14 6l6 6-6 6" />
    </template>
    <template v-else-if="name === 'logout'">
      <path d="M15 4h3.5A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5H15" />
      <path d="m10 8-4 4 4 4M6 12h9" />
    </template>
    <template v-else-if="name === 'layers'">
      <path d="m12 3 8.5 4.5L12 12 3.5 7.5z" />
      <path d="m3.5 12 8.5 4.5 8.5-4.5M3.5 16.5 12 21l8.5-4.5" />
    </template>
    <template v-else-if="name === 'door'">
      <path d="M5 20V5.5A1.5 1.5 0 0 1 6.5 4H17a2 2 0 0 1 2 2v14" />
      <path d="M3 20h18M14 12h.01" />
    </template>
    <template v-else-if="name === 'cpu'">
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <rect x="9.5" y="9.5" width="5" height="5" rx="1" />
      <path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" />
    </template>
    <template v-else-if="name === 'monitor'">
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M9 20h6M12 16v4" />
    </template>
    <template v-else-if="name === 'calendar'">
      <rect x="3.5" y="5" width="17" height="15.5" rx="2" />
      <path d="M8 3v4M16 3v4M3.5 10h17" />
    </template>
    <template v-else-if="name === 'zap'">
      <path d="M13 2 4.5 13.5H11L10 22l8.5-11.5H12z" />
    </template>
    <template v-else-if="name === 'package'">
      <path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z" />
      <path d="m4 7.5 8 4.5 8-4.5M12 12v9" />
    </template>
    <template v-else-if="name === 'database'">
      <ellipse cx="12" cy="5.5" rx="8" ry="3" />
      <path d="M4 5.5v13c0 1.7 3.6 3 8 3s8-1.3 8-3v-13M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3" />
    </template>
    <template v-else-if="name === 'sliders'">
      <path d="M4 6h16M4 12h16M4 18h16" />
      <circle cx="9" cy="6" r="2" />
      <circle cx="15" cy="12" r="2" />
      <circle cx="8" cy="18" r="2" />
    </template>
    <template v-else-if="name === 'users'">
      <circle cx="9" cy="8" r="3.2" />
      <path d="M2.8 19.5a6.2 6.2 0 0 1 12.4 0" />
      <path d="M16 5.2a3.2 3.2 0 0 1 0 5.9M17.5 14.2a5.5 5.5 0 0 1 3.7 5.3" />
    </template>
    <template v-else-if="name === 'scroll'">
      <path d="M6 4.5h11a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H7.5" />
      <path d="M6 4.5A2.5 2.5 0 0 0 3.5 7v1.5H6M4 8.5v9.5A2 2 0 0 0 6 20h.5" />
      <path d="M9.5 9h6M9.5 12.5h6M9.5 16h4" />
    </template>
    <template v-else-if="name === 'megaphone'">
      <path d="M3 11v3a1 1 0 0 0 1 1h2l5 4V6L6 10H4a1 1 0 0 0-1 1Z" />
      <path d="M15 8.5a4 4 0 0 1 0 7M18 6a7.5 7.5 0 0 1 0 12" />
    </template>
    <template v-else-if="name === 'pencil'">
      <path d="m4 20 4.5-1L20 7.5a2.1 2.1 0 0 0-3-3L5.5 16z" />
      <path d="m15 6 3 3" />
    </template>
    <template v-else-if="name === 'trash'">
      <path d="M4 7h16M9 7V4.5h6V7M6 7l1 13h10l1-13" />
      <path d="M10 11v5M14 11v5" />
    </template>
    <template v-else-if="name === 'copy'">
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3" />
    </template>
    <template v-else-if="name === 'filter'">
      <path d="M4 5h16l-6.2 7.2V19l-3.6 2v-8.8z" />
    </template>
    <template v-else-if="name === 'arrow-left'">
      <path d="M20 12H4M10 6l-6 6 6 6" />
    </template>
    <template v-else-if="name === 'chevron-left'">
      <path d="m15 6-6 6 6 6" />
    </template>
    <template v-else-if="name === 'wifi'">
      <path d="M2.5 8.5a15 15 0 0 1 19 0M5.5 12a10.5 10.5 0 0 1 13 0M8.5 15.5a6 6 0 0 1 7 0" />
      <path d="M12 19h.01" />
    </template>
    <template v-else-if="name === 'wifi-off'">
      <path
        d="m3 3 18 18M9.5 15.8a4 4 0 0 1 5 0M5.5 12a10.5 10.5 0 0 1 3.2-1.8M15.2 10.2A10.5 10.5 0 0 1 18.5 12M2.5 8.5a15 15 0 0 1 5-2.4M16.5 6.1a15 15 0 0 1 5 2.4"
      />
      <path d="M12 19h.01" />
    </template>
    <template v-else-if="name === 'upload'">
      <path d="M12 20V9m0 0 4 4m-4-4-4 4M5 4h14" />
    </template>
    <template v-else-if="name === 'key'">
      <circle cx="8" cy="8" r="4.5" />
      <path d="m11.5 11.5 8 8M16 16l2.5-2.5M18.5 18.5 21 16" />
    </template>
    <template v-else-if="name === 'terminal'">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="m7 9 3 3-3 3M13 15h4" />
    </template>
    <template v-else-if="name === 'clock'">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 1.8" />
    </template>
    <template v-else-if="name === 'external-link'">
      <path d="M14 4h6v6M20 4l-9 9" />
      <path
        d="M19 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-12A1.5 1.5 0 0 1 4 18.5v-12A1.5 1.5 0 0 1 5.5 5H10"
      />
    </template>
    <template v-else-if="name === 'more'">
      <circle cx="5" cy="12" r="1.4" />
      <circle cx="12" cy="12" r="1.4" />
      <circle cx="19" cy="12" r="1.4" />
    </template>
    <template v-else-if="name === 'pin'">
      <path d="M9 4h6l-1 6 4 4H6l4-4z" />
      <path d="M12 14v7" />
    </template>
    <template v-else-if="name === 'archive'">
      <rect x="3" y="4" width="18" height="4" rx="1" />
      <path d="M5 8v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8M10 12h4" />
    </template>
    <template v-else-if="name === 'rotate-ccw'">
      <path d="M4 11a8 8 0 1 1 2.3 6.4M4 5v6h6" />
    </template>
    <template v-else-if="name === 'play-circle'">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M10 8.5v7l6-3.5z" />
    </template>
    <template v-else-if="name === 'check-circle'">
      <circle cx="12" cy="12" r="8.5" />
      <path d="m8.5 12 2.5 2.5 4.5-5" />
    </template>
  </svg>
</template>
