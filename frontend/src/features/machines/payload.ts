//! 机器遥测 payload 的防御式解析。
//!
//! 服务端把 room-agent 的上报体原样存进 `machine_snapshots.payload`；`sections` 里的
//! 每个采集域都是可选 JSON（platform-health / platform-diagnose / qeaudit / docker /
//! virsh 的输出，以及 room-agent 内置采集明细）。真实 agent 与外部工具的字段可能
//! 增删，所以这里全部按「尽力而为」解析：读不到就返回 null / 空集合，页面显示
//! 「本次未采集」，绝不抛错。
//!
//! 已知来源结构：
//! - `platform-health/v2` 快照：`host` / `machine` / `cpu` / `memory` / `swap` /
//!   `disks[]` / `networks[]` / `components[]` / `processes` / `health`；
//! - room-agent 内置明细：`sections.disks[]` / `sections.network[]` /
//!   `sections.thermal[]` / `sections.agent` / `sections.containers[]` / `sections.vms[]`；
//! - `platform-diagnose --json`：`symptom` / `samples` / `summary` / `findings[]`；
//! - `qeaudit scan --format json`：`summary` / `findings[]`。

export type JsonObject = Record<string, unknown>

export interface HostView {
  hostname: string | null
  osName: string | null
  osVersion: string | null
  kernelVersion: string | null
  arch: string | null
  distributionId: string | null
  uptimeSecs: number | null
  bootTimeUnix: number | null
}

export interface HardwareView {
  vendor: string | null
  model: string | null
  family: string | null
  sku: string | null
  serial: string | null
  assetTag: string | null
  motherboardVendor: string | null
  motherboardName: string | null
}

export interface CpuView {
  brand: string | null
  vendor: string | null
  physicalCores: number | null
  logicalCores: number | null
  frequencyMhz: number | null
  usagePercent: number | null
  perCore: number[]
  load1: number | null
  load5: number | null
  load15: number | null
  hypervisor: string | null
}

export interface MemoryView {
  totalBytes: number | null
  usedBytes: number | null
  availableBytes: number | null
  freeBytes: number | null
  usagePercent: number | null
}

export interface SwapView {
  totalBytes: number | null
  usedBytes: number | null
  freeBytes: number | null
  usagePercent: number | null
}

export interface DiskView {
  mountPoint: string
  filesystem: string | null
  kind: string | null
  totalBytes: number | null
  usedBytes: number | null
  availableBytes: number | null
  usagePercent: number | null
  readOnly: boolean
}

export interface NetworkView {
  name: string
  state: string | null
  mac: string | null
  ipv4: string[]
  speedMbps: number | null
  rxBytes: number | null
  txBytes: number | null
  rxPackets: number | null
  txPackets: number | null
  errors: number | null
  dropped: number | null
}

export interface ThermalView {
  label: string
  temperatureC: number | null
  maxC: number | null
  criticalC: number | null
  fanRpm: number | null
}

export interface ProcessView {
  pid: number | null
  name: string
  cpuPercent: number | null
  memoryBytes: number | null
  status: string | null
  user: string | null
}

export interface ProcessesView {
  total: number | null
  byState: { label: string; count: number }[]
  top: ProcessView[]
}

export interface ContainerView {
  name: string
  image: string | null
  state: string | null
  status: string | null
  ports: string | null
}

export interface VmView {
  name: string
  state: string | null
  vcpus: number | null
  memoryBytes: number | null
}

export interface SecurityFindingView {
  id: string
  title: string
  status: string
  severity: string
  detail: string | null
  remediation: string | null
  reference: string | null
}

export interface SecuritySummaryView {
  total: number
  pass: number
  warn: number
  fail: number
  skip: number
  worstSeverity: string | null
}

export interface SecurityView {
  tool: string | null
  version: string | null
  profile: string | null
  generatedAt: string | null
  hostname: string | null
  summary: SecuritySummaryView
  findings: SecurityFindingView[]
}

export interface DiagnoseEvidenceView {
  metric: string
  observed: number | null
  threshold: number | null
  unit: string | null
  note: string | null
}

export interface DiagnoseOffenderView {
  name: string
  pid: number | null
  role: string | null
  observed: number | null
  unit: string | null
}

export interface DiagnoseFindingView {
  dimension: string
  cause: string
  confidencePercent: number | null
  summary: string
  remediation: string | null
  evidence: DiagnoseEvidenceView[]
  offenders: DiagnoseOffenderView[]
}

export interface DiagnoseView {
  symptom: string | null
  samples: number | null
  summary: string | null
  findings: DiagnoseFindingView[]
}

export interface AgentToolView {
  key: string
  label: string
  available: boolean
  disabled: boolean
  durationMs: number | null
  version: string | null
  error: string | null
}

export interface AgentView {
  version: string | null
  intervalSecs: number | null
  collectDurationMs: number | null
  machineId: string | null
  hostname: string | null
  arch: string | null
  osName: string | null
  kernelVersion: string | null
  uptimeSecs: number | null
  cpuModel: string | null
  logicalCores: number | null
  physicalCores: number | null
  load1: number | null
  load5: number | null
  load15: number | null
  /** 网卡 MAC（WOL 目标；服务端契约 `sections.agent.host.macs`，旧 agent 可能为空）。 */
  macs: string[]
  tools: AgentToolView[]
}

export type MachineIssueSeverity = 'info' | 'warning' | 'critical'

export interface MachineIssueView {
  severity: MachineIssueSeverity
  code: string
  message: string
  detail: string | null
}

export interface MachinePayloadView {
  /** `sections.platform_health` 是否存在（决定「本次未采集」提示）。 */
  platformHealthPresent: boolean
  platformHealthSchema: string | null
  host: HostView
  hardware: HardwareView
  cpu: CpuView | null
  memory: MemoryView | null
  swap: SwapView | null
  /** null = 本次未采集；[] = 已采集但没有挂载点。 */
  disks: DiskView[] | null
  networks: NetworkView[] | null
  thermal: ThermalView[] | null
  processes: ProcessesView | null
  containers: ContainerView[] | null
  vms: VmView[] | null
  security: SecurityView | null
  diagnose: DiagnoseView | null
  agent: AgentView | null
  healthIssues: MachineIssueView[]
}

// ---------------------------------------------------------------------------
// 基础取值工具：未知 JSON → 具体类型，缺失/类型不符一律 null。
// ---------------------------------------------------------------------------

export function asObject(value: unknown): JsonObject | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null
  return value as JsonObject
}

export function asArray(value: unknown): unknown[] | null {
  return Array.isArray(value) ? value : null
}

export function asString(value: unknown): string | null {
  if (typeof value === 'string') {
    const trimmed = value.trim()
    return trimmed ? trimmed : null
  }
  if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  if (typeof value === 'boolean') return value ? 'true' : 'false'
  return null
}

export function asNumber(value: unknown): number | null {
  if (typeof value === 'number') return Number.isFinite(value) ? value : null
  if (typeof value === 'string') {
    const trimmed = value.trim()
    if (!trimmed) return null
    const parsed = Number(trimmed)
    return Number.isFinite(parsed) ? parsed : null
  }
  return null
}

export function asBoolean(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value
  if (typeof value === 'number') return value !== 0
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase()
    if (normalized === 'true' || normalized === 'yes' || normalized === '1') return true
    if (normalized === 'false' || normalized === 'no' || normalized === '0') return false
  }
  return null
}

const MAC_PATTERN = /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/

/** 读取 MAC 数组（WOL 目标）：只保留合法 `aa:bb:cc:dd:ee:ff`，小写 + 去重。 */
function macArrayField(record: JsonObject | null | undefined, ...keys: string[]): string[] {
  if (!record) return []
  for (const key of keys) {
    const value = asArray(record[key])
    if (!value) continue
    const macs = value
      .map((item) => asString(item)?.toLowerCase())
      .filter((item): item is string => typeof item === 'string' && MAC_PATTERN.test(item))
    return [...new Set(macs)]
  }
  return []
}

function strField(record: JsonObject | null | undefined, ...keys: string[]): string | null {
  if (!record) return null
  for (const key of keys) {
    const value = asString(record[key])
    if (value !== null) return value
  }
  return null
}

function numField(record: JsonObject | null | undefined, ...keys: string[]): number | null {
  if (!record) return null
  for (const key of keys) {
    const value = asNumber(record[key])
    if (value !== null) return value
  }
  return null
}

function boolField(record: JsonObject | null | undefined, ...keys: string[]): boolean | null {
  if (!record) return null
  for (const key of keys) {
    const value = asBoolean(record[key])
    if (value !== null) return value
  }
  return null
}

function objField(record: JsonObject | null | undefined, ...keys: string[]): JsonObject | null {
  if (!record) return null
  for (const key of keys) {
    const value = asObject(record[key])
    if (value !== null) return value
  }
  return null
}

function arrayField(record: JsonObject | null | undefined, ...keys: string[]): unknown[] | null {
  if (!record) return null
  for (const key of keys) {
    const value = asArray(record[key])
    if (value !== null) return value
  }
  return null
}

/** 优先取第一个非空数组；全为空时返回第一个存在的数组（区分「已采集但为空」）。 */
function firstNonEmptyArray(...candidates: (unknown[] | null)[]): unknown[] | null {
  for (const candidate of candidates) {
    if (candidate !== null && candidate.length > 0) return candidate
  }
  for (const candidate of candidates) {
    if (candidate !== null) return candidate
  }
  return null
}

function clampPercent(value: number | null): number | null {
  if (value === null) return null
  return Math.min(100, Math.max(0, value))
}

/** Unix 秒 → ISO 字符串（用于 formatDateTime 系列函数）。 */
export function unixToIso(seconds: number | null | undefined): string | null {
  if (seconds === null || seconds === undefined || seconds <= 0) return null
  const date = new Date(seconds * 1000)
  return Number.isNaN(date.getTime()) ? null : date.toISOString()
}

// ---------------------------------------------------------------------------
// 各采集域解析
// ---------------------------------------------------------------------------

function parseDisk(value: unknown): DiskView | null {
  const disk = asObject(value)
  if (!disk) return null
  const totalBytes = numField(disk, 'total_bytes', 'total', 'size_bytes')
  const usedBytes = numField(disk, 'used_bytes', 'used')
  const availableBytes = numField(disk, 'available_bytes', 'available', 'free_bytes', 'free')
  let usagePercent = clampPercent(numField(disk, 'usage_percent', 'used_percent'))
  if (usagePercent === null && totalBytes !== null && usedBytes !== null && totalBytes > 0) {
    usagePercent = clampPercent((usedBytes / totalBytes) * 100)
  }
  return {
    mountPoint: strField(disk, 'mount_point', 'mount', 'path', 'name') ?? '—',
    filesystem: strField(disk, 'file_system', 'filesystem', 'fstype', 'name', 'device'),
    kind: strField(disk, 'kind', 'type', 'drive_type'),
    totalBytes,
    usedBytes,
    availableBytes,
    usagePercent,
    readOnly: boolField(disk, 'read_only', 'readonly') ?? false,
  }
}

function parseNetwork(value: unknown): NetworkView | null {
  const net = asObject(value)
  if (!net) return null
  const ipv4Raw = arrayField(net, 'ipv4', 'ipv4_addresses', 'addresses') ?? []
  const ipv4: string[] = []
  for (const item of ipv4Raw) {
    const address = asString(item)
    if (address) ipv4.push(address)
  }
  const sum = (...keys: string[]): number | null => {
    let total = 0
    let found = false
    for (const key of keys) {
      const value = numField(net, key)
      if (value !== null) {
        total += value
        found = true
      }
    }
    return found ? total : null
  }
  return {
    name: strField(net, 'name', 'interface', 'device') ?? '—',
    state: strField(net, 'state', 'status', 'operstate'),
    mac: strField(net, 'mac', 'mac_address', 'hwaddr'),
    ipv4,
    speedMbps: numField(net, 'link_speed_mbps', 'speed_mbps', 'speed'),
    rxBytes: numField(net, 'rx_bytes', 'received_bytes'),
    txBytes: numField(net, 'tx_bytes', 'sent_bytes'),
    rxPackets: numField(net, 'rx_packets', 'received_packets'),
    txPackets: numField(net, 'tx_packets', 'sent_packets'),
    errors: sum('rx_errors', 'tx_errors'),
    dropped: sum('rx_dropped', 'tx_dropped'),
  }
}

function parseThermal(value: unknown): ThermalView | null {
  const sensor = asObject(value)
  if (!sensor) return null
  return {
    label: strField(sensor, 'label', 'sensor', 'name') ?? '—',
    temperatureC: numField(sensor, 'temperature_c', 'temp_c', 'temperature'),
    maxC: numField(sensor, 'max_c', 'max_temperature_c'),
    criticalC: numField(sensor, 'critical_c', 'critical_temperature_c'),
    fanRpm: numField(sensor, 'fan_rpm', 'rpm', 'fan_speed_rpm'),
  }
}

function parseProcess(value: unknown): ProcessView | null {
  const process = asObject(value)
  if (!process) return null
  return {
    pid: numField(process, 'pid'),
    name: strField(process, 'name', 'command', 'exe') ?? '—',
    cpuPercent: clampPercent(numField(process, 'cpu_percent', 'cpu_usage', 'cpu')),
    memoryBytes: numField(process, 'memory_bytes', 'memory', 'rss_bytes'),
    status: strField(process, 'status', 'state'),
    user: strField(process, 'user', 'username', 'owner'),
  }
}

function parseProcesses(
  platformHealth: JsonObject | null,
  sections: JsonObject | null,
): ProcessesView | null {
  const source = objField(platformHealth, 'processes') ?? objField(sections, 'processes')
  const topRaw =
    arrayField(source, 'top', 'processes', 'items') ??
    arrayField(sections, 'top_processes') ??
    arrayField(platformHealth, 'top_processes')
  if (!source && !topRaw) return null

  const top: ProcessView[] = []
  for (const item of topRaw ?? []) {
    const process = parseProcess(item)
    if (process) top.push(process)
  }

  const byState: { label: string; count: number }[] = []
  const byStateRaw = objField(source, 'by_state', 'states')
  if (byStateRaw) {
    for (const [label, count] of Object.entries(byStateRaw)) {
      const parsed = asNumber(count)
      if (parsed !== null) byState.push({ label, count: parsed })
    }
  }

  return {
    total: numField(source, 'total', 'count', 'process_count'),
    byState,
    top,
  }
}

function formatPortBinding(value: unknown): string | null {
  if (typeof value === 'string') return value.trim() || null
  if (typeof value === 'number') return String(value)
  const binding = asObject(value)
  if (!binding) return null
  const hostIp = strField(binding, 'host_ip', 'HostIp', 'ip')
  const hostPort = strField(binding, 'host_port', 'HostPort', 'hostPort', 'public_port')
  const containerPort = strField(
    binding,
    'container_port',
    'ContainerPort',
    'private_port',
    'target',
  )
  const protocol = strField(binding, 'protocol', 'Protocol', 'type')
  const suffix = protocol ? `/${protocol}` : ''
  if (hostPort && containerPort) {
    return `${hostIp ? `${hostIp}:` : ''}${hostPort}->${containerPort}${suffix}`
  }
  if (containerPort) return `${containerPort}${suffix}`
  if (hostPort) return `${hostIp ? `${hostIp}:` : ''}${hostPort}`
  return null
}

/** docker `Ports` 可能是字符串、数组或 `{"80/tcp":[{HostIp,HostPort}]}` 映射。 */
function formatPorts(value: unknown): string | null {
  if (Array.isArray(value)) {
    const parts = value
      .map((item) => formatPortBinding(item))
      .filter((part): part is string => part !== null)
    return parts.length ? parts.join(', ') : null
  }
  const mapping = asObject(value)
  if (mapping) {
    const parts: string[] = []
    for (const [containerPort, bindings] of Object.entries(mapping)) {
      if (Array.isArray(bindings)) {
        for (const binding of bindings) {
          const formatted = formatPortBinding(binding)
          parts.push(formatted ? `${formatted}->${containerPort}` : containerPort)
        }
      } else {
        const formatted = formatPortBinding(bindings)
        parts.push(formatted ? `${formatted}->${containerPort}` : containerPort)
      }
    }
    return parts.length ? parts.join(', ') : null
  }
  return formatPortBinding(value)
}

function parseContainer(value: unknown): ContainerView | null {
  const container = asObject(value)
  if (!container) return null
  return {
    name: strField(container, 'name', 'Names', 'Name') ?? '—',
    image: strField(container, 'image', 'Image'),
    state: strField(container, 'state', 'State'),
    status: strField(container, 'status', 'Status'),
    ports: formatPorts(container.ports ?? container.Ports),
  }
}

function parseVm(value: unknown): VmView | null {
  const vm = asObject(value)
  if (!vm) return null
  return {
    name: strField(vm, 'name', 'Name', 'domain') ?? '—',
    state: strField(vm, 'state', 'State', 'status'),
    vcpus: numField(vm, 'vcpus', 'vcpu', 'cpu_count'),
    memoryBytes: numField(vm, 'memory_bytes', 'memory', 'max_memory_bytes'),
  }
}

function parseSecurityFinding(value: unknown): SecurityFindingView | null {
  const finding = asObject(value)
  if (!finding) return null
  const id = strField(finding, 'id', 'code', 'rule', 'check_id') ?? '—'
  return {
    id,
    title: strField(finding, 'title', 'name', 'summary', 'message') ?? id,
    status: (strField(finding, 'status', 'result') ?? 'skip').toLowerCase(),
    severity: (strField(finding, 'severity', 'risk', 'level') ?? 'info').toLowerCase(),
    detail: strField(finding, 'detail', 'description', 'message'),
    remediation: strField(finding, 'remediation', 'fix', 'advice'),
    reference: strField(finding, 'reference', 'ref', 'baseline'),
  }
}

function parseSecurity(security: JsonObject | null): SecurityView | null {
  if (!security) return null
  const raw = asObject(security.raw)
  const rawReport = asObject(raw?.report)
  const summary =
    asObject(security.summary) ?? asObject(raw?.summary) ?? asObject(rawReport?.summary)
  const findingsRaw =
    arrayField(security, 'findings', 'checks', 'results') ??
    arrayField(raw, 'findings', 'checks') ??
    arrayField(rawReport, 'findings', 'checks') ??
    []

  const findings: SecurityFindingView[] = []
  for (const item of findingsRaw) {
    const finding = parseSecurityFinding(item)
    if (finding) findings.push(finding)
  }

  if (!summary && findings.length === 0) return null

  const count = (status: string): number =>
    findings.filter((finding) => finding.status === status).length
  const summaryView: SecuritySummaryView = {
    total: numField(summary, 'total') ?? findings.length,
    pass: numField(summary, 'pass', 'passed', 'ok') ?? count('pass'),
    warn: numField(summary, 'warn', 'warning') ?? count('warn'),
    fail: numField(summary, 'fail', 'failed', 'error') ?? count('fail'),
    skip: numField(summary, 'skip', 'skipped') ?? count('skip'),
    worstSeverity: strField(summary, 'worst_severity', 'worstSeverity', 'severity'),
  }

  return {
    tool: strField(security, 'tool') ?? 'qeaudit',
    version: strField(security, 'version'),
    profile: strField(security, 'profile'),
    generatedAt: strField(security, 'generated_at'),
    hostname: strField(security, 'hostname'),
    summary: summaryView,
    findings,
  }
}

function parseDiagnoseFinding(value: unknown): DiagnoseFindingView | null {
  const finding = asObject(value)
  if (!finding) return null
  const causeRaw = finding.cause ?? finding.code ?? finding.name
  const cause =
    typeof causeRaw === 'string'
      ? causeRaw
      : (strField(asObject(causeRaw), 'code', 'name', 'kind') ?? '—')
  const confidenceRaw = numField(finding, 'confidence', 'confidence_percent', 'score')
  const evidenceRaw = arrayField(finding, 'evidence', 'signals') ?? []
  const offendersRaw = arrayField(finding, 'offenders', 'processes') ?? []
  return {
    dimension: strField(finding, 'dimension', 'layer', 'scope') ?? 'unknown',
    cause,
    confidencePercent:
      confidenceRaw === null ? null : confidenceRaw <= 1 ? confidenceRaw * 100 : confidenceRaw,
    summary: strField(finding, 'summary', 'message', 'title') ?? cause,
    remediation: strField(finding, 'remediation', 'advice', 'suggestion'),
    evidence: evidenceRaw
      .map((item) => {
        const evidence = asObject(item)
        if (!evidence) return null
        return {
          metric: strField(evidence, 'metric', 'name', 'signal') ?? '—',
          observed: numField(evidence, 'observed', 'value'),
          threshold: numField(evidence, 'threshold', 'limit'),
          unit: strField(evidence, 'unit'),
          note: strField(evidence, 'note', 'detail', 'description'),
        }
      })
      .filter((item): item is DiagnoseEvidenceView => item !== null),
    offenders: offendersRaw
      .map((item) => {
        const offender = asObject(item)
        if (!offender) return null
        return {
          name: strField(offender, 'name', 'process') ?? '—',
          pid: numField(offender, 'pid'),
          role: strField(offender, 'role'),
          observed: numField(offender, 'observed', 'value'),
          unit: strField(offender, 'unit'),
        }
      })
      .filter((item): item is DiagnoseOffenderView => item !== null),
  }
}

function parseDiagnose(diagnose: unknown, sections: JsonObject | null): DiagnoseView | null {
  if (typeof diagnose === 'string') {
    const summary = asString(diagnose)
    return summary ? { symptom: null, samples: null, summary, findings: [] } : null
  }
  const source = asObject(diagnose) ?? asObject(sections?.diagnose)
  if (!source) return null
  const findingsRaw = arrayField(source, 'findings', 'results', 'causes') ?? []
  const findings: DiagnoseFindingView[] = []
  for (const item of findingsRaw) {
    const finding = parseDiagnoseFinding(item)
    if (finding) findings.push(finding)
  }
  return {
    symptom: strField(source, 'symptom', 'mode', 'scene'),
    samples: numField(source, 'samples', 'sample_count'),
    summary: strField(source, 'summary', 'verdict', 'conclusion', 'message'),
    findings,
  }
}

const TOOL_LABELS: Record<string, string> = {
  platform_health: 'platform-health',
  platform_diagnose: 'platform-diagnose',
  qeaudit: 'qeaudit',
  docker: 'docker',
  virsh: 'virsh',
}

function parseAgent(
  agent: JsonObject | null,
  root: JsonObject,
  sections: JsonObject | null,
): AgentView | null {
  const agentSection = agent ?? asObject(sections?.agent)
  if (!agentSection) return null
  const agentHost = asObject(agentSection.host)
  const toolsRaw = asObject(agentSection.tools)
  const tools: AgentToolView[] = []
  if (toolsRaw) {
    for (const [key, raw] of Object.entries(toolsRaw)) {
      const tool = asObject(raw)
      if (!tool) continue
      tools.push({
        key,
        label: TOOL_LABELS[key] ?? key,
        available: boolField(tool, 'available', 'ok') ?? false,
        disabled: boolField(tool, 'disabled') ?? false,
        durationMs: numField(tool, 'duration_ms', 'duration'),
        version: strField(tool, 'version', 'schema'),
        error: strField(tool, 'error', 'message'),
      })
    }
  }
  return {
    version: strField(agentSection, 'agent_version', 'version') ?? strField(root, 'agent_version'),
    intervalSecs: numField(agentSection, 'interval_secs', 'interval'),
    collectDurationMs: numField(agentSection, 'builtin_collect_duration_ms', 'collect_duration_ms'),
    machineId: strField(agentHost, 'machine_id', 'machineId'),
    hostname: strField(agentHost, 'hostname'),
    arch: strField(agentHost, 'arch'),
    osName: strField(agentHost, 'os_name'),
    kernelVersion: strField(agentHost, 'kernel_version'),
    uptimeSecs: numField(agentHost, 'uptime_secs'),
    cpuModel: strField(agentHost, 'cpu_model'),
    logicalCores: numField(agentHost, 'logical_cores'),
    physicalCores: numField(agentHost, 'physical_cores'),
    load1: numField(agentHost, 'load1'),
    load5: numField(agentHost, 'load5'),
    load15: numField(agentHost, 'load15'),
    macs: macArrayField(agentHost, 'macs'),
    tools,
  }
}

function parseIssueSeverity(raw: string | null): MachineIssueSeverity {
  switch ((raw ?? '').toLowerCase()) {
    case 'critical':
    case 'crit':
    case 'fatal':
      return 'critical'
    case 'warning':
    case 'warn':
      return 'warning'
    default:
      return 'info'
  }
}

function parseIssue(value: unknown): MachineIssueView | null {
  const issue = asObject(value)
  if (!issue) return null
  const severity = parseIssueSeverity(strField(issue, 'severity', 'level', 'risk'))
  const code = strField(issue, 'code', 'id', 'rule', 'name') ?? 'unknown'
  const subject = strField(issue, 'subject', 'title', 'name')
  const observed = numField(issue, 'observed', 'value')
  const threshold = numField(issue, 'threshold', 'limit')
  const unit = strField(issue, 'unit') ?? ''
  const messageFromSource = strField(issue, 'message', 'summary', 'description')
  let message = messageFromSource
  if (!message) {
    const observedText = observed === null ? null : `${observed}${unit}`
    const thresholdText = threshold === null ? null : `${threshold}${unit}`
    if (subject && observedText) {
      message = thresholdText
        ? `${subject} 实测 ${observedText}（阈值 ${thresholdText}）`
        : `${subject} 实测 ${observedText}`
    } else {
      message = subject ?? code
    }
  }
  const detail = strField(issue, 'detail')
  return {
    severity,
    code,
    message,
    detail: detail && detail !== message ? detail : null,
  }
}

function parseHealthIssues(
  platformHealthHealth: JsonObject | null,
  rootHealth: JsonObject | null,
): MachineIssueView[] {
  const source =
    arrayField(platformHealthHealth, 'issues', 'problems') ??
    arrayField(rootHealth, 'issues', 'problems') ??
    []
  const issues: MachineIssueView[] = []
  for (const item of source) {
    const issue = parseIssue(item)
    if (issue) issues.push(issue)
  }
  return issues
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

/**
 * 把 `MachineSnapshot.payload` 解析成页面使用的视图模型。
 * 入参可以是任意 JSON；解析失败返回全空结构，而不是抛异常。
 */
/** 解析 `vm.command list` 的 `result.vms`（与遥测 sections.vms 同结构）；非法返回 null。 */
export function parseVmList(value: unknown): VmView[] | null {
  const root = asObject(value)
  if (!root) return null
  const raw = asArray(root.vms)
  if (!raw) return null
  return raw.map((item) => parseVm(item)).filter((item): item is VmView => item !== null)
}

export function parseMachinePayload(payload: unknown): MachinePayloadView {
  const root = asObject(payload) ?? {}
  const sections = asObject(root.sections)
  const platformHealth = asObject(sections?.platform_health)
  const platformHealthHost = asObject(platformHealth?.host)
  const platformHealthIdentity = asObject(platformHealth?.identity)
  const platformHealthMachine = asObject(platformHealth?.machine)
  const platformHealthCpu = objField(platformHealth, 'cpu')
  const platformHealthHealth = asObject(platformHealth?.health)
  const rootHealth = asObject(root.health)
  const rootMetrics = asObject(root.metrics)
  const agentSection = asObject(sections?.agent)
  const agentHost = asObject(agentSection?.host)
  const cpuFallback = objField(sections, 'cpu')

  const host: HostView = {
    hostname:
      strField(platformHealthIdentity, 'hostname') ??
      strField(root, 'hostname') ??
      strField(agentHost, 'hostname'),
    osName:
      strField(platformHealthHost, 'os_name') ??
      strField(root, 'os_name') ??
      strField(agentHost, 'os_name'),
    osVersion:
      strField(platformHealthHost, 'os_version') ?? strField(platformHealthHost, 'long_os_version'),
    kernelVersion:
      strField(platformHealthHost, 'kernel_version') ??
      strField(root, 'kernel_version') ??
      strField(agentHost, 'kernel_version'),
    arch: strField(platformHealthHost, 'arch') ?? strField(agentHost, 'arch'),
    distributionId: strField(platformHealthHost, 'distribution_id'),
    uptimeSecs: numField(platformHealthHost, 'uptime_secs') ?? numField(rootMetrics, 'uptime_secs'),
    bootTimeUnix: numField(platformHealthHost, 'boot_time_unix'),
  }

  const hardware: HardwareView = {
    vendor: strField(platformHealthMachine, 'vendor'),
    model: strField(platformHealthMachine, 'model'),
    family: strField(platformHealthMachine, 'family'),
    sku: strField(platformHealthMachine, 'sku'),
    serial: strField(platformHealthMachine, 'serial'),
    assetTag: strField(platformHealthMachine, 'asset_tag'),
    motherboardVendor: strField(platformHealthMachine, 'motherboard_vendor'),
    motherboardName: strField(platformHealthMachine, 'motherboard_name'),
  }

  const cpuSource = platformHealthCpu ?? cpuFallback
  const cpuBrand = strField(cpuSource, 'brand', 'model') ?? strField(agentHost, 'cpu_model')
  const cpuUsage =
    clampPercent(numField(cpuSource, 'usage_percent', 'usage')) ??
    clampPercent(numField(rootMetrics, 'cpu_usage_percent'))
  const logicalCores = numField(cpuSource, 'logical_cores') ?? numField(agentHost, 'logical_cores')
  const physicalCores =
    numField(cpuSource, 'physical_cores') ?? numField(agentHost, 'physical_cores')
  const perCoreRaw = arrayField(cpuSource, 'per_core_usage', 'per_core', 'cores') ?? []
  const perCore: number[] = []
  for (const item of perCoreRaw) {
    const value = clampPercent(asNumber(item))
    if (value !== null) perCore.push(value)
  }
  const load = objField(cpuSource, 'load_average') ?? objField(platformHealth, 'load_average')
  const load1 = numField(load, 'one', 'load1', '1') ?? numField(agentHost, 'load1')
  const load5 = numField(load, 'five', 'load5', '5') ?? numField(agentHost, 'load5')
  const load15 = numField(load, 'fifteen', 'load15', '15') ?? numField(agentHost, 'load15')
  const hypervisor = strField(objField(cpuSource, 'virtualization'), 'hypervisor')
  const cpuHasData =
    cpuBrand !== null ||
    cpuUsage !== null ||
    logicalCores !== null ||
    physicalCores !== null ||
    perCore.length > 0 ||
    load1 !== null
  const cpu: CpuView | null = cpuHasData
    ? {
        brand: cpuBrand,
        vendor: strField(cpuSource, 'vendor'),
        physicalCores,
        logicalCores,
        frequencyMhz: numField(cpuSource, 'frequency_mhz', 'frequency'),
        usagePercent: cpuUsage,
        perCore,
        load1,
        load5,
        load15,
        hypervisor,
      }
    : null

  const memorySource = objField(platformHealth, 'memory') ?? objField(sections, 'memory')
  const memoryUsage =
    clampPercent(numField(memorySource, 'usage_percent', 'usage')) ??
    clampPercent(numField(rootMetrics, 'memory_usage_percent'))
  const memory: MemoryView | null =
    memorySource || memoryUsage !== null
      ? {
          totalBytes: numField(memorySource, 'total_bytes', 'total'),
          usedBytes: numField(memorySource, 'used_bytes', 'used'),
          availableBytes: numField(memorySource, 'available_bytes', 'available', 'free_bytes'),
          freeBytes: numField(memorySource, 'free_bytes', 'free'),
          usagePercent: memoryUsage,
        }
      : null

  const swapSource = objField(platformHealth, 'swap') ?? objField(sections, 'swap')
  const swapTotal = numField(swapSource, 'total_bytes', 'total')
  const swap: SwapView | null = swapSource
    ? {
        totalBytes: swapTotal,
        usedBytes: numField(swapSource, 'used_bytes', 'used'),
        freeBytes: numField(swapSource, 'free_bytes', 'free'),
        usagePercent: clampPercent(numField(swapSource, 'usage_percent', 'usage')),
      }
    : null

  const diskSource = firstNonEmptyArray(
    arrayField(platformHealth, 'disks'),
    arrayField(sections, 'disks'),
  )
  const disks: DiskView[] | null =
    diskSource === null
      ? null
      : diskSource.map((item) => parseDisk(item)).filter((item): item is DiskView => item !== null)

  const networkSource = firstNonEmptyArray(
    arrayField(platformHealth, 'networks'),
    arrayField(sections, 'network'),
  )
  const networks: NetworkView[] | null =
    networkSource === null
      ? null
      : networkSource
          .map((item) => parseNetwork(item))
          .filter((item): item is NetworkView => item !== null)

  const thermalSource = firstNonEmptyArray(
    arrayField(platformHealth, 'components', 'temperatures', 'sensors'),
    arrayField(sections, 'thermal', 'temperatures'),
  )
  const thermal: ThermalView[] | null =
    thermalSource === null
      ? null
      : thermalSource
          .map((item) => parseThermal(item))
          .filter((item): item is ThermalView => item !== null)

  const processes = parseProcesses(platformHealth, sections)
  const processesView = processes

  const containersRaw = arrayField(sections, 'containers')
  const containers: ContainerView[] | null =
    containersRaw === null
      ? null
      : containersRaw
          .map((item) => parseContainer(item))
          .filter((item): item is ContainerView => item !== null)

  const vmsRaw = arrayField(sections, 'vms')
  const vms: VmView[] | null =
    vmsRaw === null
      ? null
      : vmsRaw.map((item) => parseVm(item)).filter((item): item is VmView => item !== null)

  const security = parseSecurity(asObject(sections?.security))
  const diagnose = parseDiagnose(sections?.diagnose, sections)
  const agent = parseAgent(agentSection, root, sections)
  const healthIssues = parseHealthIssues(platformHealthHealth, rootHealth)

  return {
    platformHealthPresent: platformHealth !== null,
    platformHealthSchema: strField(platformHealth, 'schema'),
    host,
    hardware,
    cpu,
    memory,
    swap,
    disks,
    networks,
    thermal,
    processes: processesView,
    containers,
    vms,
    security,
    diagnose,
    agent,
    healthIssues,
  }
}
