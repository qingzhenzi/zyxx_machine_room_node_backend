import { describe, expect, it } from 'vitest'

import {
  batchVmNamePreview,
  buildCreateArgs,
  isValidVmName,
  parseDiskInfo,
  parseExportResult,
  parseSnapshotList,
  parseGpuInput,
  slotSuffix,
  validateVmCreateForm,
  type VmCreateForm,
} from '../vmCommand'

function form(overrides: Partial<VmCreateForm> = {}): VmCreateForm {
  return {
    name: 'lab-1',
    vcpus: 2,
    memoryMiB: 2048,
    diskGiB: 20,
    baseVolume: 'ubuntu-22.04',
    start: true,
    gpu: '',
    ...overrides,
  }
}

describe('vmCommand 校验', () => {
  it('name 白名单与长度', () => {
    expect(isValidVmName('lab-1')).toBe(true)
    expect(isValidVmName('a.b_c-d')).toBe(true)
    expect(isValidVmName('../evil')).toBe(false)
    expect(isValidVmName('a/b')).toBe(false)
    expect(isValidVmName('a b')).toBe(false)
    expect(isValidVmName('')).toBe(false)
    expect(isValidVmName('x'.repeat(65))).toBe(false)
  })

  it('create 表单范围校验', () => {
    expect(validateVmCreateForm(form())).toBeNull()
    expect(validateVmCreateForm(form({ name: '' }))).toMatch(/名称/)
    expect(validateVmCreateForm(form({ name: 'bad name' }))).toMatch(/名称/)
    expect(validateVmCreateForm(form({ vcpus: 0 }))).toMatch(/vCPU/)
    expect(validateVmCreateForm(form({ vcpus: 65 }))).toMatch(/vCPU/)
    expect(validateVmCreateForm(form({ memoryMiB: 255 }))).toMatch(/内存/)
    expect(validateVmCreateForm(form({ memoryMiB: 262145 }))).toMatch(/内存/)
    expect(validateVmCreateForm(form({ diskGiB: 0 }))).toMatch(/磁盘/)
    expect(validateVmCreateForm(form({ diskGiB: 2049 }))).toMatch(/磁盘/)
    expect(validateVmCreateForm(form({ gpu: 'bad' }))).toMatch(/GPU/)
    expect(validateVmCreateForm(form({ gpu: '0000:01:00.0,0000:02:00.0,0000:03:00.0' }))).toMatch(
      /最多 2 个/,
    )
  })

  it('buildCreateArgs 只带有效字段', () => {
    expect(buildCreateArgs(form())).toEqual({
      name: 'lab-1',
      vcpus: 2,
      memory_mib: 2048,
      disk_gb: 20,
      start: true,
      base_volume: 'ubuntu-22.04',
    })
    expect(buildCreateArgs(form({ baseVolume: '  ', gpu: '0000:01:00.0' }))).toEqual({
      name: 'lab-1',
      vcpus: 2,
      memory_mib: 2048,
      disk_gb: 20,
      start: true,
      gpu: ['0000:01:00.0'],
    })
  })

  it('GPU 地址格式', () => {
    expect(parseGpuInput('0000:01:00.0').error).toBeNull()
    expect(parseGpuInput('0000:01:00.0, 0000:02:00.1').values).toHaveLength(2)
    expect(parseGpuInput('not-a-pci').error).toMatch(/GPU/)
    expect(parseGpuInput('0000:01:00.9').error).toMatch(/GPU/)
  })
})

describe('批量下发命名', () => {
  it('槽位尾号清洗与名称拼接', () => {
    expect(slotSuffix('room-101')).toBe('101')
    expect(slotSuffix('w3f-crit-slot-1793663233')).toBe('1793663233')
    expect(slotSuffix('room_102')).toBe('102')
    expect(slotSuffix('   ')).toBe('1')
    expect(batchVmNamePreview('lab', 'room-101')).toBe('lab-101')
    expect(batchVmNamePreview('lab', 'room-102')).toBe('lab-102')
    const name = batchVmNamePreview('p'.repeat(32), `room-${'9'.repeat(60)}`)
    expect(name.length).toBeLessThanOrEqual(64)
  })
})

describe('命令结果解析', () => {
  it('parseSnapshotList', () => {
    expect(
      parseSnapshotList({
        snapshots: [
          { tag: 'snap1', created_at: '2026-11-07 10:00:00', vm_size: '2.0G', date: '2026-11-07' },
          { name: 'snap2' },
          null,
        ],
      }),
    ).toEqual([
      { tag: 'snap1', createdAt: '2026-11-07 10:00:00', vmSize: '2.0G', date: '2026-11-07' },
      { tag: 'snap2', createdAt: null, vmSize: null, date: null },
    ])
    expect(parseSnapshotList({})).toEqual([])
    expect(parseSnapshotList(null)).toEqual([])
  })

  it('parseDiskInfo 保留 backing 链', () => {
    expect(
      parseDiskInfo({
        disks: [
          {
            path: '/data/vms/lab-1/disk.qcow2',
            format: 'qcow2',
            virtual_size: 21474836480,
            backing_path: '/pool/ubuntu-22.04.qcow2',
            backing_format: 'qcow2',
          },
        ],
        chain: ['/data/vms/lab-1/disk.qcow2', '/pool/ubuntu-22.04.qcow2'],
      }),
    ).toEqual({
      disks: [
        {
          path: '/data/vms/lab-1/disk.qcow2',
          format: 'qcow2',
          virtualSize: 21474836480,
          backingPath: '/pool/ubuntu-22.04.qcow2',
          backingFormat: 'qcow2',
        },
      ],
      chain: ['/data/vms/lab-1/disk.qcow2', '/pool/ubuntu-22.04.qcow2'],
    })
    expect(parseDiskInfo({})).toBeNull()
  })

  it('parseExportResult', () => {
    expect(
      parseExportResult({ path: '/data/exports/lab-1.qcow2', sha256: 'abc', size_bytes: 123 }),
    ).toEqual({
      path: '/data/exports/lab-1.qcow2',
      sha256: 'abc',
      sizeBytes: 123,
    })
    expect(parseExportResult({})).toBeNull()
  })
})
