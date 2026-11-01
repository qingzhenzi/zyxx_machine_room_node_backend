import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useUiStore = defineStore('ui', () => {
  const commandPaletteOpen = ref(false)
  /** 侧边栏在窄屏（1280~1440）下可以折叠，状态存内存即可。 */
  const sidebarCompact = ref(false)

  function openCommandPalette() {
    commandPaletteOpen.value = true
  }

  function toggleSidebar() {
    sidebarCompact.value = !sidebarCompact.value
  }

  return { commandPaletteOpen, sidebarCompact, openCommandPalette, toggleSidebar }
})
