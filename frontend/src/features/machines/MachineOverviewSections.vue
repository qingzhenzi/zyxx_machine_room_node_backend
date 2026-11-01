<script setup lang="ts">
import { computed } from 'vue'

import EmptyState from '@/components/ui/EmptyState.vue'
import InfoList from '@/components/ui/InfoList.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import {
  diagnoseDimensionMeta,
  securitySeverityMeta,
  securityStatusMeta,
  severityMeta,
  workloadStateMeta,
} from './machineMeta'
import type { MachinePayloadView } from './payload'
import { unixToIso } from './payload'
import { formatBytes, formatFullDateTime, formatSeconds } from '@/utils/format'

const props = defineProps<{
  payload: MachinePayloadView
}>()

const hostItems = computed(() => [
  { label: '主机名', value: props.payload.host.hostname, mono: true },
  { label: '操作系统', value: props.payload.host.osName },
  { label: '系统版本', value: props.payload.host.osVersion },
  { label: '内核', value: props.payload.host.kernelVersion, mono: true },
  { label: '架构', value: props.payload.host.arch, mono: true },
  { label: '运行时长', value: formatSeconds(props.payload.host.uptimeSecs) },
  {
    label: '启动时间',
    value: unixToIso(props.payload.host.bootTimeUnix)
      ? formatFullDateTime(unixToIso(props.payload.host.bootTimeUnix) ?? '')
      : null,
  },
  { label: '厂商', value: props.payload.hardware.vendor },
  { label: '型号', value: props.payload.hardware.model },
  { label: '序列号', value: props.payload.hardware.serial, mono: true },
  { label: '主板', value: props.payload.hardware.motherboardName },
  { label: 'Machine ID', value: props.payload.agent?.machineId, mono: true },
])

const cpu = computed(() => props.payload.cpu)
const memory = computed(() => props.payload.memory)
const swap = computed(() => props.payload.swap)
const processes = computed(() => props.payload.processes)

const diskRows = computed(() => props.payload.disks ?? [])
const networkRows = computed(() => props.payload.networks ?? [])
const thermalRows = computed(() => props.payload.thermal ?? [])
const containerRows = computed(() => props.payload.containers ?? [])
const vmRows = computed(() => props.payload.vms ?? [])
const security = computed(() => props.payload.security)
const diagnose = computed(() => props.payload.diagnose)
const agent = computed(() => props.payload.agent)

const hasCpuSection = computed(
  () => cpu.value !== null || memory.value !== null || swap.value !== null,
)
</script>

<template>
  <div class="flex flex-col gap-4">
    <PageSection title="主机信息" description="来自 platform-health 主机清单与 room-agent 内置采集">
      <div class="px-5 py-4">
        <InfoList :items="hostItems" :columns="2" />
      </div>
    </PageSection>

    <PageSection title="CPU / 内存" description="采样时刻的使用率、负载与容量">
      <div v-if="hasCpuSection" class="grid grid-cols-2 gap-4 px-5 py-4">
        <div v-if="cpu" class="rounded-lg border border-line bg-canvas/30 px-4 py-3">
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0">
              <p class="truncate text-[12px] font-medium text-ink">{{ cpu.brand ?? 'CPU' }}</p>
              <p class="mt-0.5 text-[10.5px] text-faint">
                {{ cpu.physicalCores ?? '—' }} 物理核 · {{ cpu.logicalCores ?? '—' }} 逻辑核
                <span v-if="cpu.frequencyMhz">· {{ cpu.frequencyMhz }} MHz</span>
              </p>
            </div>
            <span class="text-[18px] font-semibold tabular-nums text-ink">
              {{ cpu.usagePercent === null ? '—' : `${cpu.usagePercent.toFixed(1)}%` }}
            </span>
          </div>
          <div class="mt-3">
            <div class="h-2 overflow-hidden rounded-full bg-canvas">
              <div
                class="h-full rounded-full bg-accent transition-[width] duration-300"
                :style="{ width: `${Math.min(100, Math.max(0, cpu.usagePercent ?? 0))}%` }"
              />
            </div>
          </div>
          <dl class="mt-3 grid grid-cols-3 gap-2 text-[10.5px]">
            <div>
              <dt class="text-faint">load 1m</dt>
              <dd class="font-mono text-ink-soft">{{ cpu.load1 ?? '—' }}</dd>
            </div>
            <div>
              <dt class="text-faint">load 5m</dt>
              <dd class="font-mono text-ink-soft">{{ cpu.load5 ?? '—' }}</dd>
            </div>
            <div>
              <dt class="text-faint">load 15m</dt>
              <dd class="font-mono text-ink-soft">{{ cpu.load15 ?? '—' }}</dd>
            </div>
          </dl>
          <div v-if="cpu.perCore.length" class="mt-3 flex h-8 items-end gap-[2px]">
            <span
              v-for="(core, index) in cpu.perCore"
              :key="index"
              class="min-w-[2px] flex-1 rounded-sm bg-accent/70"
              :style="{ height: `${Math.max(4, Math.min(100, core))}%` }"
              :title="`核心 ${index}: ${core.toFixed(1)}%`"
            />
          </div>
        </div>

        <div class="flex flex-col gap-3">
          <div v-if="memory" class="rounded-lg border border-line bg-canvas/30 px-4 py-3">
            <div class="flex items-center justify-between">
              <span class="text-[11.5px] font-medium text-ink">内存</span>
              <span class="font-mono text-[12px] tabular-nums text-ink-soft">
                {{ memory.usagePercent === null ? '—' : `${memory.usagePercent.toFixed(1)}%` }}
              </span>
            </div>
            <div class="mt-2 h-2 overflow-hidden rounded-full bg-canvas">
              <div
                class="h-full rounded-full bg-sky-500"
                :style="{ width: `${Math.min(100, Math.max(0, memory.usagePercent ?? 0))}%` }"
              />
            </div>
            <p class="mt-2 text-[10.5px] text-faint">
              已用 {{ formatBytes(memory.usedBytes) }} / 共 {{ formatBytes(memory.totalBytes) }} ·
              可用 {{ formatBytes(memory.availableBytes) }}
            </p>
          </div>

          <div v-if="swap" class="rounded-lg border border-line bg-canvas/30 px-4 py-3">
            <div class="flex items-center justify-between">
              <span class="text-[11.5px] font-medium text-ink">Swap</span>
              <span class="font-mono text-[12px] tabular-nums text-ink-soft">
                {{ swap.usagePercent === null ? '—' : `${swap.usagePercent.toFixed(1)}%` }}
              </span>
            </div>
            <p class="mt-2 text-[10.5px] text-faint">
              已用 {{ formatBytes(swap.usedBytes) }} / 共 {{ formatBytes(swap.totalBytes) }}
            </p>
          </div>
        </div>
      </div>
      <EmptyState
        v-else
        title="本次未采集 CPU / 内存"
        description="platform-health 与内置采集都不可用时，本节为空。"
        icon="cpu"
      />
    </PageSection>

    <PageSection title="磁盘" description="挂载点容量与使用率">
      <div v-if="diskRows.length" class="overflow-x-auto">
        <table class="w-full border-collapse text-left">
          <thead>
            <tr class="border-b border-line bg-canvas/50">
              <th class="table-th pl-5">挂载点</th>
              <th class="table-th">文件系统</th>
              <th class="table-th text-right">总量</th>
              <th class="table-th text-right">已用</th>
              <th class="table-th w-[180px]">使用率</th>
              <th class="table-th pr-5">只读</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="disk in diskRows"
              :key="disk.mountPoint"
              class="border-b border-line/70 last:border-b-0"
            >
              <td class="table-td pl-5 font-mono text-[11.5px] text-ink">{{ disk.mountPoint }}</td>
              <td class="table-td text-[11px] text-muted">{{ disk.filesystem ?? '—' }}</td>
              <td class="table-td text-right font-mono text-[11px]">
                {{ formatBytes(disk.totalBytes) }}
              </td>
              <td class="table-td text-right font-mono text-[11px]">
                {{ formatBytes(disk.usedBytes) }}
              </td>
              <td class="table-td">
                <div class="flex items-center gap-2">
                  <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-canvas">
                    <div
                      class="h-full rounded-full"
                      :class="
                        (disk.usagePercent ?? 0) >= 90
                          ? 'bg-rose-500'
                          : (disk.usagePercent ?? 0) >= 75
                            ? 'bg-amber-500'
                            : 'bg-emerald-500'
                      "
                      :style="{ width: `${Math.min(100, Math.max(0, disk.usagePercent ?? 0))}%` }"
                    />
                  </div>
                  <span class="w-10 text-right font-mono text-[10.5px] text-muted">
                    {{ disk.usagePercent === null ? '—' : `${disk.usagePercent.toFixed(0)}%` }}
                  </span>
                </div>
              </td>
              <td class="table-td pr-5 text-[11px] text-muted">
                {{ disk.readOnly ? '是' : '否' }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else title="本次未采集磁盘信息" icon="database" />
    </PageSection>

    <div class="grid grid-cols-2 gap-4">
      <PageSection title="网络接口" description="累计流量与错误计数">
        <div v-if="networkRows.length" class="overflow-x-auto">
          <table class="w-full border-collapse text-left">
            <thead>
              <tr class="border-b border-line bg-canvas/50">
                <th class="table-th pl-5">接口</th>
                <th class="table-th">状态</th>
                <th class="table-th">IPv4</th>
                <th class="table-th pr-5 text-right">收发</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="net in networkRows"
                :key="net.name"
                class="border-b border-line/70 last:border-b-0"
              >
                <td class="table-td pl-5 font-mono text-[11px] text-ink">{{ net.name }}</td>
                <td class="table-td text-[11px] text-muted">{{ net.state ?? '—' }}</td>
                <td class="table-td font-mono text-[10.5px] text-muted">
                  {{ net.ipv4.join(', ') || '—' }}
                </td>
                <td class="table-td pr-5 text-right font-mono text-[10px] text-faint">
                  ↓{{ formatBytes(net.rxBytes) }} / ↑{{ formatBytes(net.txBytes) }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <EmptyState v-else title="本次未采集网络信息" icon="wifi" />
      </PageSection>

      <PageSection title="温度 / 风扇" description="来自 hwmon 与 platform-health 传感器">
        <ul v-if="thermalRows.length" class="divide-y divide-line/70">
          <li
            v-for="sensor in thermalRows"
            :key="sensor.label"
            class="flex items-center justify-between px-5 py-2.5"
          >
            <span class="truncate text-[11.5px] text-ink-soft">{{ sensor.label }}</span>
            <span
              class="shrink-0 font-mono text-[11px]"
              :class="(sensor.temperatureC ?? 0) >= 80 ? 'text-rose-600' : 'text-muted'"
            >
              {{ sensor.temperatureC === null ? '—' : `${sensor.temperatureC.toFixed(1)} °C` }}
            </span>
          </li>
        </ul>
        <EmptyState v-else title="本次未采集温度" icon="zap" />
      </PageSection>
    </div>

    <PageSection title="进程" description="总数与 Top 资源占用">
      <div v-if="processes" class="px-5 py-4">
        <p class="text-[11.5px] text-muted">
          当前进程总数 <span class="font-mono text-ink">{{ processes.total ?? '—' }}</span>
          <template v-if="processes.byState.length">
            · 状态分布：{{
              processes.byState.map((item) => `${item.label} ${item.count}`).join(' / ')
            }}
          </template>
        </p>
        <div v-if="processes.top.length" class="mt-3 overflow-x-auto rounded-lg border border-line">
          <table class="w-full border-collapse text-left">
            <thead>
              <tr class="border-b border-line bg-canvas/50">
                <th class="table-th pl-4">PID</th>
                <th class="table-th">进程</th>
                <th class="table-th">用户</th>
                <th class="table-th text-right">CPU</th>
                <th class="table-th pr-4 text-right">内存</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="item in processes.top"
                :key="`${item.pid}-${item.name}`"
                class="border-b border-line/70 last:border-b-0"
              >
                <td class="table-td pl-4 font-mono text-[10.5px] text-faint">
                  {{ item.pid ?? '—' }}
                </td>
                <td class="table-td truncate text-[11px] text-ink-soft">{{ item.name }}</td>
                <td class="table-td text-[10.5px] text-muted">{{ item.user ?? '—' }}</td>
                <td class="table-td text-right font-mono text-[10.5px]">
                  {{ item.cpuPercent === null ? '—' : `${item.cpuPercent.toFixed(1)}%` }}
                </td>
                <td class="table-td pr-4 text-right font-mono text-[10.5px]">
                  {{ formatBytes(item.memoryBytes) }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
      <EmptyState v-else title="本次未采集进程信息" icon="terminal" />
    </PageSection>

    <div class="grid grid-cols-2 gap-4">
      <PageSection title="容器" description="docker ps 快照">
        <ul v-if="containerRows.length" class="divide-y divide-line/70">
          <li v-for="container in containerRows" :key="container.name" class="px-5 py-2.5">
            <div class="flex items-center justify-between gap-2">
              <span class="truncate font-mono text-[11px] text-ink">{{ container.name }}</span>
              <StateBadge
                :label="workloadStateMeta(container.state).label"
                :tone="workloadStateMeta(container.state).tone"
              />
            </div>
            <p class="mt-0.5 truncate text-[10.5px] text-faint">
              {{ container.image ?? '—'
              }}<span v-if="container.status"> · {{ container.status }}</span>
            </p>
          </li>
        </ul>
        <EmptyState
          v-else
          title="没有运行中的容器"
          description="docker 不可用或没有容器时显示为空。"
          icon="package"
        />
      </PageSection>

      <PageSection title="虚拟机" description="virsh list --all 快照">
        <ul v-if="vmRows.length" class="divide-y divide-line/70">
          <li
            v-for="vm in vmRows"
            :key="vm.name"
            class="flex items-center justify-between px-5 py-2.5"
          >
            <div class="min-w-0">
              <p class="truncate font-mono text-[11px] text-ink">{{ vm.name }}</p>
              <p class="mt-0.5 text-[10.5px] text-faint">
                {{ vm.vcpus ?? '—' }} vCPU · {{ formatBytes(vm.memoryBytes) }}
              </p>
            </div>
            <StateBadge
              :label="workloadStateMeta(vm.state).label"
              :tone="workloadStateMeta(vm.state).tone"
            />
          </li>
        </ul>
        <EmptyState
          v-else
          title="没有虚拟机"
          description="libvirt 不可用或没有域时显示为空。"
          icon="server"
        />
      </PageSection>
    </div>

    <PageSection title="安全核查" description="qeaudit 基线扫描结果">
      <div v-if="security" class="px-5 py-4">
        <div class="flex flex-wrap items-center gap-2 text-[11px]">
          <StateBadge label="通过" tone="emerald" :dot="false" />
          <span class="font-mono text-ink-soft">{{ security.summary.pass }}</span>
          <StateBadge label="告警" tone="amber" :dot="false" />
          <span class="font-mono text-ink-soft">{{ security.summary.warn }}</span>
          <StateBadge label="失败" tone="rose" :dot="false" />
          <span class="font-mono text-ink-soft">{{ security.summary.fail }}</span>
          <span class="text-faint">共 {{ security.summary.total }} 项</span>
          <span v-if="security.profile" class="text-faint">· profile {{ security.profile }}</span>
          <span v-if="security.generatedAt" class="text-faint"
            >· {{ formatFullDateTime(security.generatedAt) }}</span
          >
        </div>
        <div
          v-if="security.findings.length"
          class="mt-3 overflow-x-auto rounded-lg border border-line"
        >
          <table class="w-full border-collapse text-left">
            <thead>
              <tr class="border-b border-line bg-canvas/50">
                <th class="table-th pl-4">检查项</th>
                <th class="table-th">状态</th>
                <th class="table-th">等级</th>
                <th class="table-th pr-4">说明</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="finding in security.findings"
                :key="finding.id"
                class="border-b border-line/70 last:border-b-0"
              >
                <td class="table-td pl-4">
                  <p class="text-[11px] text-ink">{{ finding.title }}</p>
                  <p class="font-mono text-[10px] text-faint">{{ finding.id }}</p>
                </td>
                <td class="table-td">
                  <StateBadge
                    :label="securityStatusMeta(finding.status).label"
                    :tone="securityStatusMeta(finding.status).tone"
                  />
                </td>
                <td class="table-td">
                  <StateBadge
                    :label="securitySeverityMeta(finding.severity).label"
                    :tone="securitySeverityMeta(finding.severity).tone"
                    :dot="false"
                  />
                </td>
                <td class="table-td pr-4 text-[10.5px] leading-4 text-muted">
                  {{ finding.detail ?? '—' }}
                  <p v-if="finding.remediation" class="mt-0.5 text-faint">
                    建议：{{ finding.remediation }}
                  </p>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-else class="mt-3 text-[11.5px] text-muted">没有需要关注的核查项。</p>
      </div>
      <EmptyState
        v-else
        title="本次未采集安全核查"
        description="qeaudit 未安装、超时或被关闭时显示为空。"
        icon="shield"
      />
    </PageSection>

    <PageSection title="诊断结论" description="platform-diagnose 根因分析">
      <div v-if="diagnose" class="px-5 py-4">
        <p class="text-[11.5px] text-muted">
          symptom <span class="font-mono text-ink-soft">{{ diagnose.symptom ?? '—' }}</span> · 样本
          {{ diagnose.samples ?? '—' }}
        </p>
        <p
          v-if="diagnose.summary"
          class="mt-2 rounded-md border border-line bg-canvas/40 px-3 py-2 text-[11.5px] leading-5 text-ink-soft"
        >
          {{ diagnose.summary }}
        </p>
        <div v-if="diagnose.findings.length" class="mt-3 flex flex-col gap-2">
          <div
            v-for="finding in diagnose.findings"
            :key="`${finding.cause}-${finding.dimension}`"
            class="rounded-lg border border-line px-4 py-3"
          >
            <div class="flex items-start justify-between gap-2">
              <div class="flex items-center gap-2">
                <StateBadge
                  :label="diagnoseDimensionMeta(finding.dimension).label"
                  :tone="diagnoseDimensionMeta(finding.dimension).tone"
                />
                <span class="font-mono text-[10.5px] text-faint">{{ finding.cause }}</span>
              </div>
              <span class="shrink-0 font-mono text-[11px] text-muted">
                {{
                  finding.confidencePercent === null
                    ? '—'
                    : `${finding.confidencePercent.toFixed(0)}%`
                }}
              </span>
            </div>
            <p class="mt-1.5 text-[11.5px] leading-5 text-ink">{{ finding.summary }}</p>
            <p v-if="finding.remediation" class="mt-1 text-[10.5px] leading-4 text-faint">
              建议：{{ finding.remediation }}
            </p>
            <ul
              v-if="finding.evidence.length"
              class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-faint"
            >
              <li v-for="evidence in finding.evidence" :key="evidence.metric">
                {{ evidence.metric }}={{ evidence.observed ?? '—' }}{{ evidence.unit ?? '' }}
              </li>
            </ul>
            <ul
              v-if="finding.offenders.length"
              class="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-muted"
            >
              <li v-for="offender in finding.offenders" :key="`${offender.name}-${offender.pid}`">
                肇事进程：{{ offender.name
                }}<span v-if="offender.pid"> (pid {{ offender.pid }})</span>
              </li>
            </ul>
          </div>
        </div>
        <p v-else class="mt-3 text-[11.5px] text-muted">未发现根因假设。</p>
      </div>
      <EmptyState
        v-else
        title="本次未采集诊断结论"
        description="platform-diagnose 未安装、超时或被关闭时显示为空。"
        icon="zap"
      />
    </PageSection>

    <PageSection title="Agent" description="采集器版本与外部工具可用性">
      <div v-if="agent" class="px-5 py-4">
        <InfoList
          :columns="2"
          :items="[
            { label: 'Agent 版本', value: `v${agent.version ?? '—'}`, mono: true },
            {
              label: '上报间隔',
              value: agent.intervalSecs === null ? null : `${agent.intervalSecs}s`,
            },
            {
              label: '采集耗时',
              value: agent.collectDurationMs === null ? null : `${agent.collectDurationMs} ms`,
              mono: true,
            },
            { label: '主机名', value: agent.hostname, mono: true },
            {
              label: 'OS / 内核',
              value: [agent.osName, agent.kernelVersion].filter(Boolean).join(' · '),
            },
            { label: 'CPU 型号', value: agent.cpuModel },
            {
              label: '逻辑核 / 物理核',
              value: `${agent.logicalCores ?? '—'} / ${agent.physicalCores ?? '—'}`,
            },
            {
              label: 'load (1/5/15)',
              value: `${agent.load1 ?? '—'} / ${agent.load5 ?? '—'} / ${agent.load15 ?? '—'}`,
              mono: true,
            },
          ]"
        />
        <div v-if="agent.tools.length" class="mt-4 overflow-x-auto rounded-lg border border-line">
          <table class="w-full border-collapse text-left">
            <thead>
              <tr class="border-b border-line bg-canvas/50">
                <th class="table-th pl-4">工具</th>
                <th class="table-th">可用</th>
                <th class="table-th text-right">耗时</th>
                <th class="table-th">版本/错误</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="tool in agent.tools"
                :key="tool.key"
                class="border-b border-line/70 last:border-b-0"
              >
                <td class="table-td pl-4 font-mono text-[10.5px] text-ink-soft">{{ tool.key }}</td>
                <td class="table-td">
                  <StateBadge
                    :label="tool.available ? '可用' : tool.disabled ? '已关闭' : '不可用'"
                    :tone="tool.available ? 'emerald' : tool.disabled ? 'slate' : 'rose'"
                    :dot="false"
                  />
                </td>
                <td class="table-td text-right font-mono text-[10.5px] text-muted">
                  {{ tool.durationMs === null ? '—' : `${tool.durationMs} ms` }}
                </td>
                <td class="table-td text-[10.5px] text-muted">
                  {{ tool.version ?? tool.error ?? '—' }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
      <EmptyState v-else title="本次未采集 Agent 信息" icon="sliders" />
    </PageSection>

    <p class="px-1 text-[10.5px] text-faint">
      健康问题严重级别说明：
      <span class="text-slate-600">提示</span> / <span class="text-amber-600">告警</span> /
      <span class="text-rose-600">严重</span>。 原始数据来自 room-agent
      上报，页面只做展示，不改变机器状态。
      <span v-if="severityMeta('info')" />
    </p>
  </div>
</template>
