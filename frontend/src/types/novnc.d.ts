/**
 * `@novnc/novnc@1.7.0` 只发布 ES module（`core/rfb.js`），没有自带 TypeScript 声明。
 * 这里只声明前端用到的 RFB 公开 API，字段/事件以 `docs/API.md` 为准。
 */
declare module '@novnc/novnc' {
  export interface RfbCredentials {
    username?: string
    password?: string
    target?: 'credentialsrequired' | 'password'
  }

  export interface RfbOptions {
    /** 连接时自动提供的凭据（libvirt VNC 只需要 password）。 */
    credentials?: RfbCredentials
    /** 多个客户端共享同一个 VNC 会话（libvirt 支持）。 */
    shared?: boolean
    /** 转发器 ID（不使用）。 */
    repeaterID?: string
    /** WebSocket 子协议（默认 `binary`）。 */
    wsProtocols?: string[]
    /** 按容器尺寸缩放画面（保持宽高比）。 */
    scaleViewport?: boolean
    /** 画面大于容器时裁剪并允许滚动。 */
    clipViewport?: boolean
    /** 请求远端把桌面分辨率调整成容器尺寸（需要服务端支持 ExtendedDesktopSize）。 */
    resizeSession?: boolean
    /** 拖拽平移画面（配合 clipViewport）。 */
    dragViewport?: boolean
    /** 使用 noVNC 自己的光标渲染（服务端不发送光标时）。 */
    showDotCursor?: boolean
    /** 只读模式（不注入键鼠）。 */
    viewOnly?: boolean
    /** 画布底色（CSS 颜色）。 */
    background?: string
    /** 压缩质量 0-9。 */
    qualityLevel?: number
    /** 压缩等级 0-9。 */
    compressionLevel?: number
    /** 触摸屏上的鼠标按键绑定。 */
    touchButton?: number
  }

  type RfbEventName =
    | 'connect'
    | 'disconnect'
    | 'credentialsrequired'
    | 'securityfailure'
    | 'clipboard'
    | 'bell'
    | 'desktopname'
    | 'capabilities'
    | 'clippingviewport'

  export default class RFB extends EventTarget {
    constructor(target: HTMLElement, urlOrChannel: string, options?: RfbOptions)

    /** 当前是否已连上 VNC 服务端。 */
    readonly connected: boolean
    /** 远端桌面名（ServerInit 之后可用）。 */
    readonly desktopName: string
    /** 当前连接状态。 */
    readonly connectionState: 'connecting' | 'connected' | 'disconnecting' | 'disconnected'
    /** 下次凭据请求时要发送的凭据。 */
    credentials: RfbCredentials
    /** 按容器尺寸自动缩放。 */
    scaleViewport: boolean
    /** 裁剪到容器尺寸。 */
    clipViewport: boolean
    /** 请求远端调整分辨率。 */
    resizeSession: boolean
    /** 只读模式。 */
    viewOnly: boolean

    /** 让画布获得键盘焦点。 */
    focus(): void
    /** 释放键盘焦点。 */
    blur(): void
    /** 主动发送凭据（credentialsrequired 回调里使用）。 */
    sendCredentials(credentials: RfbCredentials): void
    /** 注入按键（keysym 为 X11 keysym）。 */
    sendKey(keysym: number, code: string | null, down?: boolean): void
    /** 注入 Ctrl+Alt+Del。 */
    sendCtrlAltDel(): void
    /** 关闭连接并清理内部资源；多次调用是安全的。 */
    disconnect(): void
    /** 与 DOM 一致的事件监听重载。 */
    addEventListener(type: RfbEventName, listener: (event: Event) => void): void
    addEventListener(type: string, listener: (event: Event) => void): void
  }
}
