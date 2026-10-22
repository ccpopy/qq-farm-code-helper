const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const elements = {
  accountName: document.querySelector('#accountName'),
  accountsCard: document.querySelector('#accountsCard'),
  accountsCount: document.querySelector('#accountsCount'),
  autoSync: document.querySelector('#autoSync'),
  captureDirectory: document.querySelector('#captureDirectory'),
  captureDirectoryPath: document.querySelector('#captureDirectoryPath'),
  copyCaptureDirectoryButton: document.querySelector('#copyCaptureDirectoryButton'),
  cleanupButton: document.querySelector('#cleanupButton'),
  checkUpdateButton: document.querySelector('#checkUpdateButton'),
  detectQqButton: document.querySelector('#detectQqButton'),
  inlineMessage: document.querySelector('#inlineMessage'),
  installUpdateButton: document.querySelector('#installUpdateButton'),
  openCaptureDirectoryButton: document.querySelector('#openCaptureDirectoryButton'),
  plotEmpty: document.querySelector('#plotEmpty'),
  plotEmptyDetail: document.querySelector('#plotEmptyDetail'),
  plotEmptyTitle: document.querySelector('#plotEmptyTitle'),
  plotList: document.querySelector('#plotList'),
  protocolCapture: document.querySelector('#protocolCapture'),
  protocolField: document.querySelector('#protocolField'),
  protocolFlow: document.querySelector('#protocolFlow'),
  protocolFlowPort: document.querySelector('#protocolFlowPort'),
  protocolModeNotice: document.querySelector('#protocolModeNotice'),
  proxyPort: document.querySelector('#proxyPort'),
  saveButton: document.querySelector('#saveButton'),
  serverToken: document.querySelector('#serverToken'),
  serverUrl: document.querySelector('#serverUrl'),
  sessionCard: document.querySelector('#sessionCard'),
  sessionLabel: document.querySelector('#sessionLabel'),
  sessionTally: document.querySelector('#sessionTally'),
  settingsForm: document.querySelector('#settingsForm'),
  settingsCard: document.querySelector('#settingsCard'),
  startButton: document.querySelector('#startButton'),
  startButtonText: document.querySelector('#startButtonText'),
  statusDetail: document.querySelector('#statusDetail'),
  statusSignal: document.querySelector('#statusSignal'),
  statusTitle: document.querySelector('#statusTitle'),
  stopButton: document.querySelector('#stopButton'),
  syncOfficialFriends: document.querySelector('#syncOfficialFriends'),
  testButton: document.querySelector('#testButton'),
  toggleToken: document.querySelector('#toggleToken'),
  tokenHint: document.querySelector('#tokenHint'),
  toastViewport: document.querySelector('#toastViewport'),
  updateNotes: document.querySelector('#updateNotes'),
  updateProxy: document.querySelector('#updateProxy'),
  updateProxyUrl: document.querySelector('#updateProxyUrl'),
  updateProxyHint: document.querySelector('#updateProxyHint'),
  updateStatus: document.querySelector('#updateStatus'),
  updateTarget: document.querySelector('#updateTarget'),
  updateVersion: document.querySelector('#updateVersion'),
  versionChip: document.querySelector('#versionChip'),
}

const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)')
const identityPollInterval = 2000
const protocolUnlockTarget = 5

let currentPhase = 'idle'
let currentCaptureDirectory = ''
let currentSession = { active: false, target: null, detail: '' }
let currentIdentityCandidates = []
let currentIdentityError = ''
let hasIdentityResult = false
let identityRequest = null
let latestUpdate = null
let updateDownloadInProgress = false
let updateProxySaveQueue = Promise.resolve()
let preferredQqNumber = ''
let pendingSyncQq = null
let codeQqNumber = null
let protocolModeUnlocked = false
let protocolUnlockClicks = 0
let savedProtocolCapture = false
let startPending = false
let stopPending = false
let toastSequence = 0
// 本次运行中每个 QQ 的最近结果；停止代理后仍保留，方便对照哪些账号还没同步。
const accountResults = new Map()

const toastIcons = {
  success: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12 4 4L19 6"/></svg>',
  warning: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 2.8 19h18.4L12 3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>',
  error: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="m9 9 6 6"/><path d="m15 9-6 6"/></svg>',
  info: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 11v5"/><path d="M12 8h.01"/></svg>',
}
const toastDurations = { success: 4200, info: 5200, warning: 8000, error: 9000 }
const activeToasts = new Map()

function setupScrollArea() {
  const area = document.querySelector('#scrollArea')
  const viewport = document.querySelector('#scrollViewport')
  const bar = document.querySelector('#scrollBar')
  const thumb = document.querySelector('#scrollThumb')
  const banner = document.querySelector('#appBanner')
  const minThumbHeight = 28
  let idleTimer = 0
  let grabOffset = 0

  const trackHeight = () => bar.clientHeight - 2
  const overflow = () => viewport.scrollHeight - viewport.clientHeight

  function sync() {
    const distance = overflow()
    area.classList.toggle('scrollable', distance > 1)
    banner.classList.toggle('raised', viewport.scrollTop > 2)
    if (distance <= 1)
      return
    const height = Math.max(minThumbHeight, Math.round(trackHeight() * viewport.clientHeight / viewport.scrollHeight))
    thumb.style.height = `${height}px`
    thumb.style.transform = `translateY(${Math.round((trackHeight() - height) * viewport.scrollTop / distance)}px)`
  }

  function scrollToRatio(ratio) {
    viewport.scrollTop = Math.min(Math.max(ratio, 0), 1) * overflow()
  }

  viewport.addEventListener('scroll', () => {
    sync()
    area.classList.add('scrolling')
    clearTimeout(idleTimer)
    idleTimer = setTimeout(() => area.classList.remove('scrolling'), 700)
  }, { passive: true })

  thumb.addEventListener('pointerdown', (event) => {
    event.preventDefault()
    thumb.setPointerCapture(event.pointerId)
    grabOffset = event.clientY - thumb.getBoundingClientRect().top
    area.classList.add('dragging')
  })
  thumb.addEventListener('pointermove', (event) => {
    if (!area.classList.contains('dragging'))
      return
    scrollToRatio((event.clientY - grabOffset - bar.getBoundingClientRect().top - 1) / (trackHeight() - thumb.offsetHeight))
  })
  thumb.addEventListener('lostpointercapture', () => area.classList.remove('dragging'))

  bar.addEventListener('pointerdown', (event) => {
    if (event.target === thumb)
      return
    const page = viewport.clientHeight * 0.9
    viewport.scrollBy({
      top: event.clientY < thumb.getBoundingClientRect().top ? -page : page,
      behavior: reducedMotion.matches ? 'auto' : 'smooth',
    })
  })

  const observer = new ResizeObserver(sync)
  observer.observe(viewport)
  observer.observe(viewport.firstElementChild)
  sync()

  return {
    reset() {
      viewport.scrollTop = 0
      sync()
    },
  }
}

const scrollArea = setupScrollArea()

function isProtocolMode() {
  return protocolModeUnlocked && elements.protocolCapture.checked
}

function settingsPayload() {
  return {
    settings: {
      server_url: elements.serverUrl.value.trim(),
      account_name: elements.accountName.value.trim(),
      qq_number: preferredQqNumber,
      auto_sync: elements.autoSync.checked,
      sync_official_friends: elements.syncOfficialFriends.checked,
      proxy_port: Number(elements.proxyPort.value),
      protocol_capture: isProtocolMode(),
      update_proxy: elements.updateProxy.checked,
      update_proxy_url: elements.updateProxyUrl.value.trim(),
    },
    token: elements.serverToken.value.trim() || null,
  }
}

function setBusy(button, busy, busyText) {
  if (!button.dataset.originalText)
    button.dataset.originalText = button.textContent
  button.disabled = busy
  button.textContent = busy ? busyText : button.dataset.originalText
}

function toastDefaultTitle(type) {
  return {
    success: '操作成功',
    warning: '需要注意',
    error: '操作未完成',
    info: '状态提示',
  }[type] || '状态提示'
}

function dismissToast(id) {
  const entry = activeToasts.get(id)
  if (!entry || entry.closing)
    return
  entry.closing = true
  window.clearTimeout(entry.timer)
  entry.node.classList.remove('visible')
  entry.node.classList.add('leaving')
  activeToasts.delete(id)
  window.setTimeout(() => entry.node.remove(), 220)
}

function scheduleToast(entry, duration) {
  window.clearTimeout(entry.timer)
  if (duration > 0)
    entry.timer = window.setTimeout(() => dismissToast(entry.id), duration)
}

function showToast(message, { type = 'info', title, duration, id } = {}) {
  if (!message)
    return null
  if (!toastIcons[type])
    type = 'info'
  const toastId = id || `toast-${++toastSequence}`
  const timeout = duration ?? toastDurations[type]
  const existing = activeToasts.get(toastId)
  if (existing) {
    existing.node.dataset.type = type
    existing.node.setAttribute('role', type === 'error' || type === 'warning' ? 'alert' : 'status')
    existing.icon.innerHTML = toastIcons[type]
    existing.title.textContent = title || toastDefaultTitle(type)
    existing.message.textContent = message
    existing.node.classList.remove('refreshing')
    void existing.node.offsetWidth
    existing.node.classList.add('refreshing')
    scheduleToast(existing, timeout)
    return toastId
  }

  if (activeToasts.size >= 3)
    dismissToast(activeToasts.keys().next().value)

  const toast = document.createElement('article')
  toast.className = 'app-toast'
  toast.dataset.type = type
  toast.setAttribute('role', type === 'error' || type === 'warning' ? 'alert' : 'status')

  const icon = document.createElement('span')
  icon.className = 'toast-icon'
  icon.innerHTML = toastIcons[type]

  const copy = document.createElement('div')
  copy.className = 'toast-copy'
  const titleElement = document.createElement('strong')
  titleElement.textContent = title || toastDefaultTitle(type)
  const messageElement = document.createElement('p')
  messageElement.textContent = message
  copy.append(titleElement, messageElement)

  const closeButton = document.createElement('button')
  closeButton.className = 'toast-close'
  closeButton.type = 'button'
  closeButton.setAttribute('aria-label', '关闭提示')
  closeButton.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m7 7 10 10"/><path d="m17 7-10 10"/></svg>'
  closeButton.addEventListener('click', () => dismissToast(toastId))

  toast.append(icon, copy, closeButton)
  elements.toastViewport.append(toast)
  const entry = {
    closing: false,
    icon,
    id: toastId,
    message: messageElement,
    node: toast,
    timer: 0,
    title: titleElement,
  }
  activeToasts.set(toastId, entry)
  toast.addEventListener('mouseenter', () => window.clearTimeout(entry.timer))
  toast.addEventListener('mouseleave', () => scheduleToast(entry, Math.min(timeout, 2400)))
  window.requestAnimationFrame(() => toast.classList.add('visible'))
  scheduleToast(entry, timeout)
  return toastId
}

function showMessage(message, { type = 'success', title, toast = true, duration, id } = {}) {
  elements.inlineMessage.textContent = message || ''
  elements.inlineMessage.classList.toggle('error', type === 'error')
  elements.inlineMessage.classList.toggle('warning', type === 'warning')
  if (message && toast)
    showToast(message, { type, title, duration, id })
}

function fillSettings(data) {
  elements.serverUrl.value = data.settings.server_url || ''
  elements.accountName.value = data.settings.account_name || 'Windows QQ'
  preferredQqNumber = data.settings.qq_number || ''
  elements.autoSync.checked = data.settings.auto_sync !== false
  elements.syncOfficialFriends.checked = data.settings.sync_official_friends !== false
  elements.proxyPort.value = data.settings.proxy_port || 8899
  savedProtocolCapture = data.settings.protocol_capture === true
  elements.protocolCapture.checked = protocolModeUnlocked && savedProtocolCapture
  elements.updateProxy.checked = data.settings.update_proxy !== false
  elements.updateProxyUrl.value = data.settings.update_proxy_url || 'https://ghproxy.net/'
  syncDeveloperControls()
  syncUpdateProxyHint()
  elements.tokenHint.textContent = data.token_configured
    ? 'Token 已安全保存在 Windows 凭据管理器；留空即保留。'
    : 'Token 保存在 Windows 凭据管理器，不写入配置文件。'
  syncTaskUi()
}

function syncDeveloperControls() {
  elements.protocolField.classList.toggle('hidden', !protocolModeUnlocked)
  elements.settingsForm.classList.toggle('protocol-locked', !protocolModeUnlocked)
  elements.versionChip.classList.toggle('developer-unlocked', protocolModeUnlocked)
  elements.versionChip.setAttribute('aria-pressed', String(protocolModeUnlocked))
}

function handleVersionChipClick() {
  if (protocolModeUnlocked)
    return

  protocolUnlockClicks += 1
  const remaining = protocolUnlockTarget - protocolUnlockClicks
  if (remaining > 0) {
    showToast(`再点击 ${remaining} 次显示协议模式`, {
      type: 'info',
      title: '开发者设置',
      duration: 2600,
      id: 'protocol-unlock',
    })
    return
  }

  protocolModeUnlocked = true
  elements.protocolCapture.checked = savedProtocolCapture
  syncDeveloperControls()
  syncTaskUi()
  renderAccounts()
  showToast('协议模式已显示', {
    type: 'success',
    title: '开发者设置',
    duration: 4200,
    id: 'protocol-unlock',
  })
}

function syncModePresentation() {
  const protocolMode = isProtocolMode()
  const listening = currentPhase === 'protocol_listening'
  elements.settingsCard.classList.toggle('protocol-enabled', protocolMode)
  elements.sessionCard.classList.toggle('protocol-enabled', protocolMode)
  elements.protocolField.classList.toggle('active', protocolMode)
  elements.protocolModeNotice.classList.toggle('hidden', !protocolMode)
  elements.protocolFlow.classList.toggle('hidden', !protocolMode)
  elements.protocolFlow.classList.toggle('listening', listening)
  elements.protocolFlowPort.textContent = String(elements.proxyPort.value || 8899)
  elements.startButton.classList.toggle('hidden', !protocolMode && currentSession.active)
  elements.stopButton.classList.toggle('hidden', !currentSession.active)
  elements.sessionLabel.textContent = protocolMode ? '协议模式' : '网络接管'
}

function syncTaskUi() {
  const isActive = currentSession.active || startPending || currentPhase === 'preparing_proxy'
  const protocolMode = isProtocolMode()
  for (const element of [
    elements.accountName,
    elements.autoSync,
    elements.syncOfficialFriends,
    elements.serverToken,
    elements.serverUrl,
    elements.testButton,
    elements.toggleToken,
  ])
    element.disabled = isActive || protocolMode
  for (const element of [elements.protocolCapture, elements.proxyPort, elements.saveButton])
    element.disabled = isActive
  elements.syncOfficialFriends.disabled = isActive || protocolMode || !elements.autoSync.checked
  elements.startButton.disabled = isActive
  elements.startButtonText.textContent = startPending || currentPhase === 'preparing_proxy'
    ? '正在启动…'
    : (isActive && protocolMode ? '正在监听…' : (protocolMode ? '启动协议监听' : '启动代理'))
  elements.stopButton.disabled = !currentSession.active || currentPhase === 'preparing_proxy' || stopPending
  elements.cleanupButton.disabled = startPending
    || stopPending
    || currentPhase === 'preparing_proxy'
  syncModePresentation()
}

function syncUpdateProxyHint() {
  elements.updateProxy.disabled = updateDownloadInProgress
  elements.updateProxyUrl.disabled = updateDownloadInProgress || !elements.updateProxy.checked
  elements.updateProxyHint.textContent = elements.updateProxy.checked
    ? '修改后自动保存；留空恢复默认地址。执行前仍校验 GitHub 官方 SHA-256。'
    : '直接从 GitHub 下载，执行前校验 GitHub 官方 SHA-256。'
}

async function persistUpdateProxyPreference() {
  const enabled = elements.updateProxy.checked
  const proxyUrl = elements.updateProxyUrl.value
  const request = updateProxySaveQueue.catch(() => {}).then(() => invoke('save_update_proxy', { enabled, proxyUrl }))
  updateProxySaveQueue = request
  const normalized = await request
  if (elements.updateProxyUrl.value === proxyUrl)
    elements.updateProxyUrl.value = normalized
  return { useProxy: enabled, proxyUrl: normalized }
}

async function saveUpdateProxyPreference() {
  syncUpdateProxyHint()
  try {
    await persistUpdateProxyPreference()
    elements.updateProxyUrl.removeAttribute('aria-invalid')
  }
  catch (error) {
    elements.updateProxyUrl.setAttribute('aria-invalid', 'true')
    showToast(String(error), { type: 'warning', title: '更新下载设置未保存', id: 'update-proxy-setting' })
  }
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0)
    return '未知大小'
  if (bytes >= 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${Math.ceil(bytes / 1024)} KB`
}

function releaseNotesSummary(notes) {
  return String(notes || '')
    .split(/\r?\n/)
    .map(line => line.replace(/^#{1,6}\s*/, '').replace(/^[-*]\s*/, '').replaceAll('`', '').trim())
    .filter(Boolean)
    .slice(0, 3)
    .join(' · ')
}

function renderUpdate(info) {
  latestUpdate = info
  const currentVersion = `v${info.currentVersion}`
  const latestVersion = `v${info.latestVersion}`
  elements.versionChip.textContent = currentVersion
  elements.updateVersion.textContent = info.updateAvailable ? `${currentVersion} → ${latestVersion}` : currentVersion
  elements.updateVersion.classList.toggle('available', info.updateAvailable)
  elements.installUpdateButton.classList.toggle('hidden', !info.updateAvailable)

  if (info.updateAvailable) {
    elements.updateStatus.textContent = `发现 ${latestVersion}，更新包 ${formatBytes(info.packageSize)}。`
    elements.updateTarget.textContent = info.installMode === 'portable'
      ? `更新时将退出程序并原地替换：${info.installTarget}`
      : `更新时将退出程序并静默安装到：${info.installTarget}`
    const summary = releaseNotesSummary(info.releaseNotes)
    elements.updateNotes.textContent = summary
    elements.updateNotes.classList.toggle('hidden', !summary)
  }
  else {
    elements.updateStatus.textContent = `当前已是最新稳定版 ${currentVersion}。`
    elements.updateTarget.textContent = `后续更新仍会保留当前路径：${info.installTarget}`
    elements.updateNotes.textContent = ''
    elements.updateNotes.classList.add('hidden')
  }
}

async function checkForUpdate({ silent = false } = {}) {
  setBusy(elements.checkUpdateButton, true, '检查中…')
  elements.updateStatus.textContent = '正在连接 GitHub Release…'
  try {
    const info = await invoke('check_for_update')
    renderUpdate(info)
    if (info.updateAvailable) {
      showToast(`发现 v${info.latestVersion}，可下载后在当前目录静默更新。`, {
        type: 'info',
        title: '发现新版本',
        duration: 8000,
        id: 'app-update',
      })
    }
    else if (!silent) {
      showToast(`当前已是最新版 v${info.currentVersion}。`, { type: 'success', title: '检查完成', id: 'app-update' })
    }
    return info
  }
  catch (error) {
    elements.updateStatus.textContent = '暂时无法检查更新，可以稍后手动重试。'
    elements.updateTarget.textContent = String(error)
    if (!silent)
      showMessage(String(error), { type: 'error', title: '检查更新失败' })
    return null
  }
  finally {
    setBusy(elements.checkUpdateButton, false)
  }
}

async function installUpdate() {
  if (updateDownloadInProgress)
    return
  if (!latestUpdate?.updateAvailable) {
    const info = await checkForUpdate()
    if (!info?.updateAvailable)
      return
  }

  setBusy(elements.installUpdateButton, true, '下载并校验中…')
  updateDownloadInProgress = true
  syncUpdateProxyHint()
  elements.checkUpdateButton.disabled = true
  elements.updateStatus.textContent = elements.updateProxy.checked
    ? '正在通过 GitHub 加速地址下载并校验更新包…'
    : '正在直接从 GitHub 下载并校验更新包…'
  elements.updateTarget.textContent = '校验成功后程序会自动退出、停止同路径进程并完成替换。'
  try {
    const preference = await persistUpdateProxyPreference()
    await invoke('install_update', preference)
    elements.updateStatus.textContent = '更新包已验证，正在退出并安装…'
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '自动更新失败', duration: 10000 })
    elements.updateStatus.textContent = '更新未执行，当前版本保持不变。'
    elements.updateTarget.textContent = String(error)
    setBusy(elements.installUpdateButton, false)
    updateDownloadInProgress = false
    syncUpdateProxyHint()
    elements.checkUpdateButton.disabled = false
  }
}

const plotGlyphs = {
  seed: '<svg viewBox="0 0 20 20" aria-hidden="true"><ellipse cx="10" cy="11" rx="3.6" ry="4.6"/></svg>',
  sprout: '<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 17V9"/><path d="M10 12c-3.4 0-5.4-2-5.8-5 3.2-.3 5.3 1 5.8 3.8"/><path d="M10 10c.4-3 2.6-4.6 5.8-4.2-.1 3-2.2 5-5.8 5"/></svg>',
  ripe: '<svg viewBox="0 0 20 20" aria-hidden="true"><path class="fill" d="M10 16c-4.2 0-6.4-3-6-7.6C8.6 8 11 10 10 16Z"/><path class="fill" d="M10 16c-.6-5.4 2-8.6 6-8.2.4 4.8-2 8.2-6 8.2Z"/><path d="M10 18v-2"/></svg>',
  wilted: '<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 17V8"/><path d="M10 8c-1-2.6-3.4-3.6-6-2.8.8 2.8 3 4.2 6 3.4"/><path d="M10 11c2.8.2 4.6 1.8 4.8 4.6-2.8.3-4.6-1.2-4.8-4.2"/></svg>',
}

function formatClock(date) {
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

function detailLines(text) {
  return String(text || '').split(/\r?\n/).map(line => line.trim()).filter(Boolean)
}

function rememberAccountResult(status) {
  const qq = status.target_qq
  if (!qq)
    return
  const lines = detailLines(status.detail)
  const identity = identityByQqNumber(qq) || accountResults.get(qq)?.identity
  if (status.phase === 'completed') {
    const profile = status.profile
    const summary = []
    if (profile?.gid)
      summary.push(`GID ${profile.gid}`)
    summary.push(...lines.filter(line => line.includes('好友')))
    accountResults.set(qq, {
      state: profile ? 'success' : 'captured',
      label: profile ? '已同步' : 'Code 可复制',
      detail: summary.join(' · ') || lines[0] || '',
      at: new Date(),
      identity,
    })
    preferredQqNumber = qq
  }
  else if (status.phase === 'error' || status.phase === 'identity_changed') {
    accountResults.set(qq, {
      state: 'failed',
      label: status.phase === 'identity_changed' ? '未同步' : '同步失败',
      detail: lines.filter(line => line !== 'Code 可复制').join(' · '),
      at: new Date(),
      identity,
    })
  }
}

function liveAccountState(qq) {
  if (pendingSyncQq === qq) {
    return {
      state: 'working',
      label: currentSession.active ? '切换中' : '正在启动',
      detail: currentSession.active ? '正在确认账号' : '正在安装临时证书，请在弹窗中确认',
    }
  }
  if (!currentSession.active || currentSession.target !== qq)
    return null
  if (currentPhase === 'waiting_login') {
    return {
      state: 'waiting',
      label: '等待打开农场',
      detail: detailLines(currentSession.detail).slice(1).join(' · ') || '在这个 QQ 里打开农场',
    }
  }
  if (currentPhase === 'code_captured' || currentPhase === 'syncing') {
    return {
      state: 'working',
      label: currentPhase === 'syncing' ? '正在同步' : '已获取 Code',
      detail: detailLines(currentSession.detail).join(' · '),
    }
  }
  return null
}

function accountBusy() {
  return startPending || Boolean(pendingSyncQq)
    || ['preparing_proxy', 'code_captured', 'syncing'].includes(currentPhase)
}

function plotRows() {
  const rows = currentIdentityCandidates.map(identity => ({ identity, online: true }))
  for (const [qq, result] of accountResults) {
    if (!rows.some(row => row.identity.qqNumber === qq))
      rows.push({ identity: result.identity || { qqNumber: qq, nickname: '', avatarUrl: '' }, online: false })
  }
  return rows
}

function plotStepIndex(phase) {
  return { waiting_login: 0, code_captured: 1, syncing: 2 }[phase] ?? -1
}

function buildPlotRow({ identity, online }) {
  const qq = identity.qqNumber
  const live = liveAccountState(qq)
  const result = accountResults.get(qq)
  const view = live || result || { state: 'idle', label: '未同步', detail: '' }
  const glyph = {
    idle: 'seed',
    waiting: 'sprout',
    working: 'sprout',
    success: 'ripe',
    captured: 'ripe',
    failed: 'wilted',
  }[view.state]
  const name = identity.nickname || '未命名'

  const item = document.createElement('li')
  item.className = 'plot'
  item.dataset.state = view.state
  if (!online)
    item.classList.add('offline')

  const avatar = document.createElement('span')
  avatar.className = 'plot-avatar'
  avatar.textContent = Array.from(name)[0]
  if (identity.avatarUrl) {
    const image = document.createElement('img')
    image.alt = ''
    image.referrerPolicy = 'no-referrer'
    image.src = identity.avatarUrl
    image.addEventListener('error', () => image.remove())
    avatar.append(image)
  }
  const badge = document.createElement('span')
  badge.className = 'plot-glyph'
  badge.innerHTML = plotGlyphs[glyph]
  avatar.append(badge)

  const who = document.createElement('div')
  who.className = 'plot-who'
  const nameElement = document.createElement('strong')
  nameElement.textContent = name
  const qqElement = document.createElement('span')
  qqElement.textContent = online ? `QQ ${qq}` : `QQ ${qq} · 已退出登录`
  who.append(nameElement, qqElement)

  const state = document.createElement('div')
  state.className = 'plot-state'
  const label = document.createElement('b')
  label.textContent = view.label
  if (result && !live && result.at) {
    const time = document.createElement('time')
    time.textContent = formatClock(result.at)
    label.append(time)
  }
  const detail = document.createElement('small')
  detail.textContent = view.detail || ''
  detail.title = view.detail || ''
  state.append(label, detail)

  const actions = document.createElement('div')
  actions.className = 'plot-actions'
  const protocolMode = isProtocolMode()
  const busy = accountBusy()
  if (view.state === 'waiting') {
    actions.append(plotButton('取消', 'plot-secondary', `取消同步 ${name}`, () => cancelAccountSync(), stopPending))
  }
  else if (view.state !== 'working') {
    const verb = elements.autoSync.checked ? '同步' : '获取 Code'
    const again = result ? `重新${elements.autoSync.checked ? '同步' : '获取'}` : verb
    const disabled = !online || busy || protocolMode || stopPending
    actions.append(plotButton(again, result ? 'plot-secondary' : 'plot-primary', `${again} ${name}`, () => syncAccount(qq), disabled))
  }
  if (codeQqNumber === qq && !live)
    actions.append(plotButton('复制 Code', 'plot-secondary', `复制 ${name} 的 Code`, copyCode, false))

  item.append(avatar, who, state, actions)

  const active = live && pendingSyncQq !== qq ? plotStepIndex(currentPhase) : -1
  if (active >= 0) {
    const steps = document.createElement('ol')
    steps.className = 'plot-steps'
    for (const [index, text] of ['打开农场', '获取 Code', elements.autoSync.checked ? '同步到服务器' : '可复制'].entries()) {
      const step = document.createElement('li')
      step.textContent = text
      step.classList.toggle('done', index < active)
      step.classList.toggle('active', index === active)
      steps.append(step)
    }
    item.append(steps)
  }
  return item
}

function plotButton(text, className, ariaLabel, onClick, disabled) {
  const button = document.createElement('button')
  button.type = 'button'
  button.className = className
  button.textContent = text
  button.setAttribute('aria-label', ariaLabel)
  button.disabled = disabled
  button.addEventListener('click', onClick)
  return button
}

function renderAccounts() {
  const protocolMode = isProtocolMode()
  elements.accountsCard.classList.toggle('hidden', protocolMode)
  if (protocolMode)
    return
  const rows = plotRows()
  const focusedLabel = document.activeElement?.closest('#plotList') ? document.activeElement.getAttribute('aria-label') : null
  elements.plotList.replaceChildren(...rows.map(buildPlotRow))
  if (focusedLabel) {
    const next = [...elements.plotList.querySelectorAll('button')].find(button => button.getAttribute('aria-label') === focusedLabel)
    next?.focus()
  }
  elements.accountsCount.textContent = currentIdentityCandidates.length ? String(currentIdentityCandidates.length) : ''
  elements.plotEmpty.classList.toggle('hidden', rows.length > 0)
  if (!rows.length) {
    elements.plotEmptyTitle.textContent = hasIdentityResult ? '没有检测到已登录的 Windows QQ' : '正在读取 Windows QQ'
    elements.plotEmptyDetail.textContent = hasIdentityResult
      ? (currentIdentityError || '登录 QQ 后会自动出现在这里，无需刷新。')
      : '登录后的 QQ 会自动出现在这里。'
  }
  renderSessionTally()
}

function renderSessionTally() {
  const synced = [...accountResults.values()].filter(result => result.state === 'success').length
  const total = currentIdentityCandidates.length
  const show = !isProtocolMode() && synced > 0
  elements.sessionTally.classList.toggle('hidden', !show)
  if (show)
    elements.sessionTally.textContent = total ? `已同步 ${synced} / ${total}` : `已同步 ${synced}`
}

function showStatusToast(status) {
  const config = {
    completed: { type: 'success', duration: 6000 },
    stopped: { type: 'info', duration: 4200 },
    protocol_listening: { type: 'success', duration: 8000 },
    identity_changed: { type: 'warning', duration: 9000 },
    error: { type: 'error', duration: 10000 },
  }[status.phase]
  if (!config)
    return
  showToast(status.detail || status.title, {
    ...config,
    id: `capture-status-${status.phase}`,
    title: status.title,
  })
}

function sessionCopy(status) {
  if (isProtocolMode() || !['waiting_login', 'code_captured', 'syncing', 'completed', 'error', 'identity_changed', 'ready'].includes(status.phase) || !status.session_active)
    return { title: status.title, detail: status.detail }
  const port = elements.proxyPort.value || 8899
  const target = status.target_qq ? identityByQqNumber(status.target_qq) : null
  const who = target ? `${target.nickname || '未命名'}（QQ ${target.qqNumber}）` : (status.target_qq ? `QQ ${status.target_qq}` : '')
  const detail = {
    ready: '选择账号开始同步',
    waiting_login: who ? `${who} · 在这个 QQ 里打开农场` : '在所选 QQ 里打开农场',
    code_captured: who ? `${who} · 已获取 Code` : '已获取 Code',
    syncing: who ? `${who} · 正在同步` : '正在同步',
  }[status.phase] || '可以选择下一个账号继续同步'
  return { title: `代理运行中 · 端口 ${port}`, detail }
}

function renderStatus(status, { notify = false } = {}) {
  const phase = status.phase || 'idle'
  const previousPhase = currentPhase
  const previousDetail = currentSession.detail
  currentPhase = phase
  currentSession = {
    active: Boolean(status.session_active),
    target: status.target_qq || null,
    detail: status.detail || '',
  }
  if (status.code_available && status.target_qq)
    codeQqNumber = status.target_qq
  else if (!status.code_available)
    codeQqNumber = null
  if (notify && (phase !== previousPhase || status.detail !== previousDetail))
    rememberAccountResult(status)
  const isActive = currentSession.active || phase === 'preparing_proxy'
  const isError = phase === 'error' || phase === 'identity_changed'
  const copy = sessionCopy(status)
  elements.statusTitle.textContent = copy.title
  elements.statusDetail.textContent = copy.detail
  currentCaptureDirectory = status.capture_directory || ''
  elements.captureDirectory.classList.toggle('hidden', !currentCaptureDirectory)
  elements.captureDirectoryPath.textContent = currentCaptureDirectory
  elements.captureDirectoryPath.title = currentCaptureDirectory
  elements.statusSignal.className = `status-signal ${isActive ? 'active' : ''} ${isError && !isActive ? 'error' : ''}`
  elements.sessionCard.classList.toggle('running', isActive)
  renderAccounts()
  syncTaskUi()
  if (notify && (phase !== previousPhase || status.detail !== previousDetail))
    showStatusToast(status)
}

async function saveSettings() {
  setBusy(elements.saveButton, true, '保存中…')
  showMessage('')
  try {
    const result = await invoke('save_settings', settingsPayload())
    fillSettings(result)
    elements.serverToken.value = ''
    const message = isProtocolMode()
      ? '协议模式设置已保存。'
      : '服务器地址、同步选项、更新下载方式和本地设置均已更新。'
    showMessage(message, { type: 'success', title: '设置已保存' })
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '保存设置失败' })
  }
  finally {
    setBusy(elements.saveButton, false)
    syncTaskUi()
  }
}

async function testConnection() {
  setBusy(elements.testButton, true, '测试中…')
  showMessage('')
  try {
    const payload = settingsPayload()
    const result = await invoke('test_connection', {
      serverUrl: payload.settings.server_url,
      token: payload.token,
    })
    showMessage(`已连接到服务器，当前用户：${result.username}（${result.role}）。`, { type: 'success', title: '服务器连接正常' })
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '服务器连接失败' })
  }
  finally {
    setBusy(elements.testButton, false)
    syncTaskUi()
  }
}

async function startCapture() {
  if (startPending || currentSession.active || stopPending)
    return
  showMessage('')
  startPending = true
  renderAccounts()
  syncTaskUi()
  try {
    await invoke('save_settings', settingsPayload())
    elements.serverToken.value = ''
    await invoke('start_capture')
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: isProtocolMode() ? '无法启动协议监听' : '无法启动代理', duration: 10000 })
  }
  finally {
    startPending = false
    renderAccounts()
    syncTaskUi()
  }
}

async function syncAccount(qqNumber) {
  if (pendingSyncQq || startPending || stopPending)
    return
  showMessage('')
  const firstStart = !currentSession.active
  pendingSyncQq = qqNumber
  startPending = firstStart
  renderAccounts()
  syncTaskUi()
  try {
    if (firstStart) {
      preferredQqNumber = qqNumber
      await invoke('save_settings', settingsPayload())
      elements.serverToken.value = ''
    }
    await invoke('sync_account', { qqNumber })
    dismissToast('identity-unavailable')
  }
  catch (error) {
    const identity = identityByQqNumber(qqNumber)
    const title = firstStart ? '无法启动同步' : `无法切换到 ${identity?.nickname || `QQ ${qqNumber}`}`
    showMessage(String(error), { type: 'error', title, duration: 10000 })
  }
  finally {
    pendingSyncQq = null
    startPending = false
    renderAccounts()
    syncTaskUi()
  }
}

async function cancelAccountSync() {
  try {
    await invoke('cancel_account_sync')
  }
  catch (error) {
    showToast(String(error), { type: 'warning', title: '无法取消' })
  }
}

async function stopCapture() {
  if (stopPending)
    return
  stopPending = true
  setBusy(elements.stopButton, true, '正在恢复…')
  syncTaskUi()
  renderAccounts()
  try {
    await invoke('stop_capture')
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '停止失败' })
  }
  finally {
    stopPending = false
    setBusy(elements.stopButton, false)
    syncTaskUi()
    renderAccounts()
  }
}

async function cleanupNetwork() {
  setBusy(elements.cleanupButton, true, '清理中…')
  try {
    await invoke('cleanup_network')
    showMessage('系统代理与临时证书均已恢复并清理。', { type: 'success', title: '网络环境已清理' })
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '网络清理失败' })
  }
  finally {
    setBusy(elements.cleanupButton, false)
  }
}

async function copyCode() {
  try {
    const code = await invoke('get_captured_code')
    await navigator.clipboard.writeText(code)
    showMessage('Code 已复制到剪贴板，请尽快使用。', { type: 'success', title: '复制成功' })
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '复制 Code 失败' })
  }
}

async function copyCaptureDirectory() {
  if (!currentCaptureDirectory)
    return
  try {
    await navigator.clipboard.writeText(currentCaptureDirectory)
    showToast('完整日志路径已复制。', { type: 'success', title: '复制成功', id: 'capture-directory' })
  }
  catch (error) {
    showToast(String(error), { type: 'error', title: '复制路径失败' })
  }
}

async function openCaptureDirectory() {
  if (!currentCaptureDirectory)
    return
  setBusy(elements.openCaptureDirectoryButton, true, '打开中…')
  try {
    await invoke('open_capture_directory')
  }
  catch (error) {
    showToast(String(error), { type: 'error', title: '打开目录失败' })
  }
  finally {
    setBusy(elements.openCaptureDirectoryButton, false)
  }
}

function identityByQqNumber(qqNumber) {
  return currentIdentityCandidates.find(identity => identity.qqNumber === qqNumber) || null
}

function sameAccounts(next) {
  return next.length === currentIdentityCandidates.length
    && next.every((identity, index) => {
      const current = currentIdentityCandidates[index]
      return identity.qqNumber === current.qqNumber
        && identity.nickname === current.nickname
        && identity.avatarUrl === current.avatarUrl
    })
}

async function detectLocalQq({ notify = true } = {}) {
  if (notify)
    elements.detectQqButton.classList.add('spinning')
  try {
    if (!identityRequest)
      identityRequest = invoke('detect_local_qqs').finally(() => { identityRequest = null })
    const identities = await identityRequest
    const next = Array.isArray(identities) ? identities : []
    const changed = !hasIdentityResult || currentIdentityError || !sameAccounts(next)
    hasIdentityResult = true
    currentIdentityError = ''
    currentIdentityCandidates = next
    if (changed)
      renderAccounts()
    if (notify)
      showToast(next.length ? `检测到 ${next.length} 个已登录的 QQ。` : '没有检测到已登录的 Windows QQ。', { type: next.length ? 'success' : 'warning', title: '账号列表已刷新', id: 'identity-unavailable' })
  }
  catch (error) {
    const changed = !hasIdentityResult || currentIdentityError !== String(error) || currentIdentityCandidates.length
    hasIdentityResult = true
    currentIdentityCandidates = []
    currentIdentityError = String(error)
    if (changed)
      renderAccounts()
    if (notify)
      showMessage(String(error), { type: 'warning', title: '未检测到可确认的 QQ', duration: 9000, id: 'identity-unavailable' })
  }
  finally {
    if (notify)
      elements.detectQqButton.classList.remove('spinning')
  }
}

function startIdentityMonitor() {
  const refresh = () => {
    if (!document.hidden)
      void detectLocalQq({ notify: false })
  }
  window.setInterval(refresh, identityPollInterval)
  window.addEventListener('focus', refresh)
  document.addEventListener('visibilitychange', refresh)
}

async function bootstrap() {
  scrollArea.reset()
  await listen('capture-status', event => renderStatus(event.payload, { notify: true }))
  const data = await invoke('get_bootstrap')
  fillSettings(data)
  renderStatus(data.status)
  await detectLocalQq({ notify: false })
  if (data.startup_warning)
    showMessage(data.startup_warning, { type: 'error', title: '启动时发现网络恢复问题', duration: 10000 })
  startIdentityMonitor()
  void checkForUpdate({ silent: true })
}

elements.saveButton.addEventListener('click', saveSettings)
elements.testButton.addEventListener('click', testConnection)
elements.startButton.addEventListener('click', startCapture)
elements.copyCaptureDirectoryButton.addEventListener('click', copyCaptureDirectory)
elements.openCaptureDirectoryButton.addEventListener('click', openCaptureDirectory)
elements.stopButton.addEventListener('click', stopCapture)
elements.cleanupButton.addEventListener('click', cleanupNetwork)
elements.checkUpdateButton.addEventListener('click', () => checkForUpdate())
elements.detectQqButton.addEventListener('click', () => detectLocalQq())
elements.installUpdateButton.addEventListener('click', installUpdate)
elements.versionChip.addEventListener('click', handleVersionChipClick)
elements.updateProxy.addEventListener('change', saveUpdateProxyPreference)
elements.updateProxyUrl.addEventListener('change', saveUpdateProxyPreference)
elements.updateProxyUrl.addEventListener('input', () => elements.updateProxyUrl.removeAttribute('aria-invalid'))
elements.updateProxyUrl.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') {
    event.preventDefault()
    elements.updateProxyUrl.blur()
  }
})
elements.autoSync.addEventListener('change', () => {
  syncTaskUi()
  renderAccounts()
})
elements.protocolCapture.addEventListener('change', () => {
  savedProtocolCapture = elements.protocolCapture.checked
  syncTaskUi()
  renderAccounts()
  showMessage('')
})
elements.proxyPort.addEventListener('input', () => {
  if (isProtocolMode())
    syncModePresentation()
})
elements.toggleToken.addEventListener('click', () => {
  const hidden = elements.serverToken.type === 'password'
  elements.serverToken.type = hidden ? 'text' : 'password'
  elements.toggleToken.textContent = hidden ? '隐藏' : '显示'
})
bootstrap().catch(error => showMessage(String(error), { type: 'error', title: '应用初始化失败', duration: 10000 }))
