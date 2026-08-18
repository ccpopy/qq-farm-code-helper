const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const elements = {
  accountName: document.querySelector('#accountName'),
  autoSync: document.querySelector('#autoSync'),
  cleanupButton: document.querySelector('#cleanupButton'),
  checkUpdateButton: document.querySelector('#checkUpdateButton'),
  copyButton: document.querySelector('#copyButton'),
  detectQqButton: document.querySelector('#detectQqButton'),
  inlineMessage: document.querySelector('#inlineMessage'),
  installUpdateButton: document.querySelector('#installUpdateButton'),
  privacyNote: document.querySelector('#privacyNote'),
  profileCard: document.querySelector('#profileCard'),
  protocolCapture: document.querySelector('#protocolCapture'),
  protocolField: document.querySelector('#protocolField'),
  protocolFlow: document.querySelector('#protocolFlow'),
  protocolModeNotice: document.querySelector('#protocolModeNotice'),
  proxyPort: document.querySelector('#proxyPort'),
  qqNumber: document.querySelector('#qqNumber'),
  profileAccountPicker: document.querySelector('#profileAccountPicker'),
  profileAvatar: document.querySelector('#profileAvatar'),
  profileFallback: document.querySelector('#profileFallback'),
  profileGid: document.querySelector('#profileGid'),
  profileIdentity: document.querySelector('#profileIdentity'),
  profileName: document.querySelector('#profileName'),
  profileNote: document.querySelector('#profileNote'),
  profileOpenId: document.querySelector('#profileOpenId'),
  profileState: document.querySelector('#profileState'),
  profileStaticIdentity: document.querySelector('#profileStaticIdentity'),
  saveButton: document.querySelector('#saveButton'),
  serverToken: document.querySelector('#serverToken'),
  serverUrl: document.querySelector('#serverUrl'),
  settingsCard: document.querySelector('#settingsCard'),
  stageList: [...document.querySelectorAll('#stageList li')],
  startButton: document.querySelector('#startButton'),
  startButtonText: document.querySelector('#startButtonText'),
  statusCode: document.querySelector('#statusCode'),
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
  updateProxyHint: document.querySelector('#updateProxyHint'),
  updateStatus: document.querySelector('#updateStatus'),
  updateTarget: document.querySelector('#updateTarget'),
  updateVersion: document.querySelector('#updateVersion'),
  versionChip: document.querySelector('#versionChip'),
}

const stageOrder = ['preparing_proxy', 'waiting_login', 'code_captured', 'syncing']
const activePhases = new Set([
  'preparing_proxy',
  'waiting_login',
  'code_captured',
  'syncing',
  'protocol_listening',
])
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)')
const identityPollInterval = 2000

let currentPhase = 'idle'
let currentIdentityCandidates = []
let currentLocalIdentity = null
let currentIdentityError = ''
let currentRemoteProfile = null
let hasIdentityResult = false
let identityRequest = null
let identitySelectionPending = false
let latestUpdate = null
let preferredQqNumber = ''
let startPending = false
let stopPending = false
let toastSequence = 0

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

function settingsPayload() {
  return {
    settings: {
      server_url: elements.serverUrl.value.trim(),
      account_name: elements.accountName.value.trim(),
      qq_number: elements.qqNumber.value.trim(),
      auto_sync: elements.autoSync.checked,
      sync_official_friends: elements.syncOfficialFriends.checked,
      proxy_port: Number(elements.proxyPort.value),
      protocol_capture: elements.protocolCapture.checked,
      update_proxy: elements.updateProxy.checked,
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
  elements.protocolCapture.checked = data.settings.protocol_capture === true
  elements.updateProxy.checked = data.settings.update_proxy !== false
  syncUpdateProxyHint()
  elements.tokenHint.textContent = data.token_configured
    ? 'Token 已安全保存在 Windows 凭据管理器；留空即保留。'
    : 'Token 保存在 Windows 凭据管理器，不写入配置文件。'
  syncTaskUi()
}

function syncModePresentation() {
  const protocolMode = elements.protocolCapture.checked
  const listening = currentPhase === 'protocol_listening'
  elements.settingsCard.classList.toggle('protocol-enabled', protocolMode)
  elements.profileCard.classList.toggle('protocol-enabled', protocolMode)
  elements.protocolField.classList.toggle('active', protocolMode)
  elements.protocolModeNotice.classList.toggle('hidden', !protocolMode)
  elements.stageList[0]?.parentElement.classList.toggle('hidden', protocolMode)
  elements.protocolFlow.classList.toggle('hidden', !protocolMode)
  elements.protocolFlow.classList.toggle('listening', listening)
  elements.privacyNote.textContent = protocolMode
    ? '协议模式只在本机保存完整网关消息，不提取或上传 Code；停止后自动恢复系统代理并移除临时证书。'
    : 'Code 与官方好友 GID 只在任务期间保存在内存中；结束后自动恢复系统代理并移除临时证书。'
}

function syncTaskUi() {
  const isActive = activePhases.has(currentPhase) || startPending
  const protocolMode = elements.protocolCapture.checked
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
  const canChooseQq = !protocolMode && !startPending && (!isActive || currentPhase === 'waiting_login')
  elements.detectQqButton.disabled = !canChooseQq || identitySelectionPending
  elements.qqNumber.disabled = !canChooseQq
    || identitySelectionPending
    || currentIdentityCandidates.length === 0
  elements.syncOfficialFriends.disabled = isActive || protocolMode || !elements.autoSync.checked
  elements.startButton.disabled = isActive
  elements.stopButton.disabled = !isActive
    || currentPhase === 'preparing_proxy'
    || stopPending
  elements.cleanupButton.disabled = startPending
    || stopPending
    || currentPhase === 'preparing_proxy'
  if (protocolMode)
    elements.startButtonText.textContent = isActive ? '正在监听…' : '启动协议监听'
  else {
    const codeActive = ['preparing_proxy', 'waiting_login', 'code_captured', 'syncing'].includes(currentPhase)
    elements.startButtonText.textContent = codeActive ? '正在获取…' : '启动获取'
  }
  syncModePresentation()
}

function syncUpdateProxyHint() {
  elements.updateProxyHint.textContent = elements.updateProxy.checked
    ? '通过 gh.lessdo.top 加速下载，执行前仍校验 GitHub 官方 SHA-256。'
    : '直接从 GitHub 下载，执行前校验 GitHub 官方 SHA-256。'
}

async function saveUpdateProxyPreference() {
  syncUpdateProxyHint()
  try {
    await invoke('save_update_proxy', { enabled: elements.updateProxy.checked })
  }
  catch (error) {
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
  if (!latestUpdate?.updateAvailable) {
    const info = await checkForUpdate()
    if (!info?.updateAvailable)
      return
  }

  setBusy(elements.installUpdateButton, true, '下载并校验中…')
  elements.checkUpdateButton.disabled = true
  elements.updateStatus.textContent = elements.updateProxy.checked
    ? '正在通过 GitHub 加速地址下载并校验更新包…'
    : '正在直接从 GitHub 下载并校验更新包…'
  elements.updateTarget.textContent = '校验成功后程序会自动退出、停止同路径进程并完成替换。'
  try {
    await invoke('save_update_proxy', { enabled: elements.updateProxy.checked })
    await invoke('install_update', { useProxy: elements.updateProxy.checked })
    elements.updateStatus.textContent = '更新包已验证，正在退出并安装…'
  }
  catch (error) {
    showMessage(String(error), { type: 'error', title: '自动更新失败', duration: 10000 })
    elements.updateStatus.textContent = '更新未执行，当前版本保持不变。'
    elements.updateTarget.textContent = String(error)
    setBusy(elements.installUpdateButton, false)
    elements.checkUpdateButton.disabled = false
  }
}

function renderProfile(profile) {
  const accountCount = currentIdentityCandidates.length > 1
    ? ` · ${currentIdentityCandidates.length} 个`
    : ''
  elements.profileState.textContent = `${profile.running ? '运行中' : '已同步'}${accountCount}`
  elements.profileState.classList.add('ready')
  elements.profileName.textContent = profile.nickname || profile.accountName || `远程账号 #${profile.accountId}`
  elements.profileIdentity.textContent = profile.qqNumber
    ? `QQ ${profile.qqNumber}`
    : `远程账号 #${profile.accountId}`
  elements.profileGid.textContent = profile.gid || '等待登录回填'
  elements.profileOpenId.textContent = profile.openId
    ? `${profile.openId.slice(0, 8)}${profile.openId.length > 8 ? '…' : ''}`
    : '等待登录回填'
  elements.profileNote.textContent = '昵称、GID 与头像来自远程登录结果；Code 流量本身不包含真实 QQ 号。'
  setProfileAvatar(profile.avatarUrl)
}

function clearProfileAvatar() {
  elements.profileAvatar.removeAttribute('src')
  elements.profileAvatar.classList.remove('visible')
  elements.profileFallback.classList.remove('hidden')
}

function setProfileAvatar(avatarUrl) {
  clearProfileAvatar()
  if (avatarUrl) {
    elements.profileAvatar.src = avatarUrl
    elements.profileAvatar.classList.add('visible')
    elements.profileFallback.classList.add('hidden')
  }
}

function renderLocalIdentity(identity) {
  elements.profileState.textContent = currentIdentityCandidates.length > 1
    ? `${currentIdentityCandidates.length} 个账号`
    : '本机账号已确认'
  elements.profileState.classList.add('ready')
  elements.profileName.textContent = identity.nickname || 'Windows QQ'
  elements.profileIdentity.textContent = `QQ ${identity.qqNumber}`
  elements.profileGid.textContent = '同步后回填'
  elements.profileOpenId.textContent = '同步后回填'
  const switchHint = currentIdentityCandidates.length > 1 ? '；可在账号下拉框切换其他账号' : ''
  elements.profileNote.textContent = `已锁定“${identity.nickname || '未命名'}”（QQ ${identity.qqNumber}）${switchHint}，捕获后会再次复核。`
  setProfileAvatar(identity.avatarUrl)
}

function renderPendingIdentity() {
  elements.qqNumber.value = ''
  elements.profileState.textContent = '正在确认'
  elements.profileState.classList.remove('ready')
  elements.profileName.textContent = '正在读取当前 Windows QQ'
  elements.profileIdentity.textContent = '不会沿用上一次的账号'
  elements.profileGid.textContent = '—'
  elements.profileOpenId.textContent = '—'
  elements.profileNote.textContent = '请保持新版 QQ 主窗口打开，检测完成前不会绑定 QQ 号。'
  clearProfileAvatar()
}

function renderUnconfirmedIdentity(error) {
  elements.profileState.textContent = currentIdentityCandidates.length > 1
    ? `请选择 · ${currentIdentityCandidates.length} 个`
    : '未确认'
  elements.profileState.classList.remove('ready')
  elements.profileName.textContent = '未确认当前 Windows QQ'
  elements.profileIdentity.textContent = '本次不会绑定 QQ 号'
  elements.profileGid.textContent = '—'
  elements.profileOpenId.textContent = '—'
  elements.profileNote.textContent = error || '当前账号无法唯一确认，已清除旧身份显示。'
  clearProfileAvatar()
}

function renderProtocolAccount() {
  elements.profileState.textContent = '仅限本机'
  elements.profileState.classList.add('ready')
  elements.profileName.textContent = '协议审查模式'
  elements.profileIdentity.textContent = '无需绑定服务器账号'
  elements.profileGid.textContent = '不读取'
  elements.profileOpenId.textContent = '不读取'
  elements.profileNote.textContent = '登录握手只透明转发给官方网关；Helper 不提取 Code，服务器同步链路不会启动。'
  clearProfileAvatar()
}

function renderAccountCard() {
  if (elements.protocolCapture.checked) {
    renderProtocolAccount()
    return
  }
  if (!hasIdentityResult) {
    renderPendingIdentity()
    return
  }
  if (!currentLocalIdentity) {
    renderUnconfirmedIdentity(currentIdentityError)
    return
  }
  const remoteMatchesLocal = currentRemoteProfile?.qqNumber
    && currentRemoteProfile.qqNumber === currentLocalIdentity.qqNumber
  if (currentPhase === 'completed' && remoteMatchesLocal)
    renderProfile(currentRemoteProfile)
  else
    renderLocalIdentity(currentLocalIdentity)
}

function updateStages(phase) {
  const activeIndex = stageOrder.indexOf(phase)
  elements.stageList.forEach((item) => {
    const index = stageOrder.indexOf(item.dataset.stage)
    item.classList.toggle('active', index === activeIndex)
    item.classList.toggle('done', activeIndex > index || phase === 'completed')
  })
}

function showStatusToast(status) {
  if (status.phase === 'waiting_login') {
    const confirmed = status.title === 'QQ 已确认，等待农场登录'
    const switched = status.detail?.includes('切换')
    showToast(status.detail || status.title, {
      type: confirmed ? (switched ? 'warning' : 'success') : 'warning',
      duration: confirmed ? 6000 : 9000,
      id: 'capture-status-waiting_login',
      title: status.title,
    })
    return
  }
  const config = {
    code_captured: { type: 'success', duration: 6500 },
    syncing: { type: 'info', duration: 5200 },
    protocol_listening: { type: 'success', duration: 8000 },
    completed: { type: 'success', duration: 6000 },
    stopped: { type: 'info', duration: 4200 },
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

function renderStatus(status, { notify = false } = {}) {
  const phase = status.phase || 'idle'
  const previousPhase = currentPhase
  const previousDetail = elements.statusDetail.textContent
  currentPhase = phase
  currentRemoteProfile = status.profile || null
  const isActive = activePhases.has(phase)
  const isError = phase === 'error' || phase === 'identity_changed'
  const isSuccess = phase === 'completed'
  elements.statusTitle.textContent = status.title
  elements.statusDetail.textContent = status.detail
  elements.statusCode.textContent = phase.replaceAll('_', ' ').toUpperCase()
  elements.statusSignal.className = `status-signal ${isActive ? 'active' : ''} ${isSuccess ? 'success' : ''} ${isError ? 'error' : ''}`
  elements.startButton.disabled = isActive
  elements.stopButton.disabled = !isActive || phase === 'preparing_proxy'
  elements.copyButton.classList.toggle('hidden', !status.code_available)
  updateStages(phase)
  renderAccountCard()
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
    const message = elements.protocolCapture.checked
      ? '协议模式与本地代理端口已保存；启动时不会校验服务器或上传 Code。'
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
  showMessage('')
  startPending = true
  syncTaskUi()
  try {
    await invoke('save_settings', settingsPayload())
    elements.serverToken.value = ''
    await invoke('start_capture')
    dismissToast('identity-unavailable')
  }
  catch (error) {
    const title = elements.protocolCapture.checked ? '无法启动协议监听' : '无法启动获取'
    showMessage(String(error), { type: 'error', title, duration: 10000 })
  }
  finally {
    startPending = false
    syncTaskUi()
  }
}

async function stopCapture() {
  if (stopPending)
    return
  stopPending = true
  setBusy(elements.stopButton, true, '正在恢复…')
  syncTaskUi()
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

function identityByQqNumber(qqNumber) {
  return currentIdentityCandidates.find(identity => identity.qqNumber === qqNumber) || null
}

function preferredDetectedQqNumber(identities) {
  if (identities.length === 1)
    return identities[0].qqNumber
  for (const qqNumber of [elements.qqNumber.value, currentLocalIdentity?.qqNumber, preferredQqNumber]) {
    if (qqNumber && identities.some(identity => identity.qqNumber === qqNumber))
      return qqNumber
  }
  return ''
}

function renderQqOptions(identities, selectedQqNumber) {
  const showPicker = identities.length !== 1
  elements.profileAccountPicker.classList.toggle('hidden', !showPicker)
  elements.profileStaticIdentity.classList.toggle('hidden', showPicker)
  const placeholder = document.createElement('option')
  placeholder.value = ''
  placeholder.textContent = identities.length > 1
    ? `检测到 ${identities.length} 个账号，请选择`
    : '等待检测 Windows QQ'
  placeholder.disabled = identities.length > 0
  const options = identities.map((identity) => {
    const option = document.createElement('option')
    option.value = identity.qqNumber
    option.textContent = `${identity.nickname || '未命名'}（QQ ${identity.qqNumber}）`
    return option
  })
  elements.qqNumber.replaceChildren(placeholder, ...options)
  elements.qqNumber.value = selectedQqNumber
}

async function confirmLocalQqSelection(qqNumber, { notify = true, render = true } = {}) {
  if (!qqNumber) {
    currentLocalIdentity = null
    currentIdentityError = currentIdentityCandidates.length > 1
      ? `检测到 ${currentIdentityCandidates.length} 个 Windows QQ 账号，请选择本次进入农场的账号。`
      : '尚未检测到可确认的 Windows QQ。'
    if (render)
      renderAccountCard()
    return null
  }

  identitySelectionPending = true
  syncTaskUi()
  try {
    const identity = await invoke('select_local_qq', { qqNumber })
    currentLocalIdentity = identity
    currentIdentityError = ''
    preferredQqNumber = identity.qqNumber
    elements.qqNumber.value = identity.qqNumber
    dismissToast('identity-unavailable')
    if (render)
      renderAccountCard()
    if (notify) {
      showMessage(`本次已锁定：${identity.nickname || '未命名'}（QQ ${identity.qqNumber}）。`, {
        type: 'success',
        title: '已选择 Windows QQ',
      })
    }
    return identity
  }
  catch (error) {
    currentLocalIdentity = null
    currentIdentityError = String(error)
    if (render)
      renderAccountCard()
    if (notify)
      showMessage(String(error), { type: 'warning', title: 'QQ 账号选择失败', duration: 9000 })
    return null
  }
  finally {
    identitySelectionPending = false
    syncTaskUi()
  }
}

async function detectLocalQq({ notify = true, render = true } = {}) {
  if (notify)
    setBusy(elements.detectQqButton, true, '检测中…')
  try {
    if (!identityRequest)
      identityRequest = invoke('detect_local_qqs').finally(() => { identityRequest = null })
    const identities = await identityRequest
    hasIdentityResult = true
    currentIdentityCandidates = Array.isArray(identities) ? identities : []
    const selectedQqNumber = preferredDetectedQqNumber(currentIdentityCandidates)
    renderQqOptions(currentIdentityCandidates, selectedQqNumber)

    if (!selectedQqNumber) {
      currentLocalIdentity = null
      currentIdentityError = `检测到 ${currentIdentityCandidates.length} 个 Windows QQ 账号，请在下拉列表选择本次进入农场的账号。`
      if (render)
        renderAccountCard()
      if (notify)
        showMessage(currentIdentityError, { type: 'warning', title: '请选择 Windows QQ', duration: 9000 })
      return null
    }

    const selectedIdentity = identityByQqNumber(selectedQqNumber)
    if (currentLocalIdentity?.qqNumber === selectedQqNumber) {
      currentLocalIdentity = selectedIdentity
      currentIdentityError = ''
      if (render)
        renderAccountCard()
      if (notify)
        showMessage(`当前账号：${selectedIdentity.nickname || '未命名'}（QQ ${selectedIdentity.qqNumber}）。`, { type: 'success', title: '已确认 Windows QQ' })
      return selectedIdentity
    }
    return confirmLocalQqSelection(selectedQqNumber, { notify, render })
  }
  catch (error) {
    hasIdentityResult = true
    currentIdentityCandidates = []
    currentLocalIdentity = null
    currentIdentityError = String(error)
    renderQqOptions([], '')
    if (render)
      renderAccountCard()
    if (notify)
      showMessage(String(error), { type: 'warning', title: '未检测到可确认的 QQ', duration: 9000, id: 'identity-unavailable' })
    return null
  }
  finally {
    if (notify)
      setBusy(elements.detectQqButton, false)
    syncTaskUi()
  }
}

function startIdentityMonitor() {
  const refresh = async () => {
    if (document.hidden)
      return
    const previous = currentLocalIdentity
    const previousCandidateCount = currentIdentityCandidates.length
    const identity = await detectLocalQq({ notify: false })
    if (previous && !identity) {
      showToast(currentIdentityError || '请保持新版 QQ 主窗口打开。', {
        type: 'warning',
        title: 'QQ 主程序已无法确认',
        duration: 9000,
        id: 'identity-state',
      })
    }
    else if (!previous && identity) {
      showToast(`当前账号：${identity.nickname || '未命名'}（QQ ${identity.qqNumber}）。`, {
        type: 'success',
        title: '已检测到 Windows QQ',
        id: 'identity-state',
      })
    }
    else if (previous && identity && previous.qqNumber !== identity.qqNumber) {
      showToast(`已从 QQ ${previous.qqNumber} 切换为 ${identity.nickname || '新账号'}（QQ ${identity.qqNumber}）。`, {
        type: 'warning',
        title: '检测到 QQ 账号切换',
        duration: 8000,
        id: 'identity-state',
      })
    }
    else if (identity && previousCandidateCount <= 1 && currentIdentityCandidates.length > 1) {
      showToast(`检测到 ${currentIdentityCandidates.length} 个 Windows QQ，当前仍锁定 QQ ${identity.qqNumber}；如需使用其他账号，请在下拉列表切换。`, {
        type: 'info',
        title: '可选择其他 QQ 账号',
        duration: 7000,
        id: 'identity-state',
      })
    }
    else if (!identity && currentIdentityCandidates.length > 1 && previousCandidateCount !== currentIdentityCandidates.length) {
      showToast(currentIdentityError, {
        type: 'warning',
        title: '请选择 Windows QQ',
        duration: 9000,
        id: 'identity-state',
      })
    }
  }
  window.setInterval(() => { void refresh() }, identityPollInterval)
  window.addEventListener('focus', () => { void refresh() })
  document.addEventListener('visibilitychange', () => { void refresh() })
}

async function bootstrap() {
  scrollArea.reset()
  await listen('capture-status', event => renderStatus(event.payload, { notify: true }))
  const data = await invoke('get_bootstrap')
  fillSettings(data)
  renderStatus(data.status)
  const initialIdentity = await detectLocalQq({ notify: false })
  if (!initialIdentity && !elements.protocolCapture.checked) {
    showMessage(`${currentIdentityError || '尚未检测到 Windows QQ。'} 你仍可先点击“启动获取”启动代理，随后再登录 QQ。`, {
      type: 'warning',
      toast: false,
    })
  }
  if (data.startup_warning)
    showMessage(data.startup_warning, { type: 'error', title: '启动时发现网络恢复问题', duration: 10000 })
  startIdentityMonitor()
  void checkForUpdate({ silent: true })
}

elements.saveButton.addEventListener('click', saveSettings)
elements.testButton.addEventListener('click', testConnection)
elements.startButton.addEventListener('click', startCapture)
elements.stopButton.addEventListener('click', stopCapture)
elements.cleanupButton.addEventListener('click', cleanupNetwork)
elements.checkUpdateButton.addEventListener('click', () => checkForUpdate())
elements.copyButton.addEventListener('click', copyCode)
elements.detectQqButton.addEventListener('click', () => detectLocalQq())
elements.installUpdateButton.addEventListener('click', installUpdate)
elements.updateProxy.addEventListener('change', saveUpdateProxyPreference)
elements.autoSync.addEventListener('change', syncTaskUi)
elements.protocolCapture.addEventListener('change', () => {
  syncTaskUi()
  renderAccountCard()
  showMessage('')
})
elements.toggleToken.addEventListener('click', () => {
  const hidden = elements.serverToken.type === 'password'
  elements.serverToken.type = hidden ? 'text' : 'password'
  elements.toggleToken.textContent = hidden ? '隐藏' : '显示'
})
elements.qqNumber.addEventListener('change', () => {
  void confirmLocalQqSelection(elements.qqNumber.value)
})
elements.profileAvatar.addEventListener('error', () => {
  elements.profileAvatar.classList.remove('visible')
  elements.profileFallback.classList.remove('hidden')
})

bootstrap().catch(error => showMessage(String(error), { type: 'error', title: '应用初始化失败', duration: 10000 }))
