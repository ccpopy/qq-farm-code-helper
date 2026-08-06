const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const elements = {
  accountName: document.querySelector('#accountName'),
  autoSync: document.querySelector('#autoSync'),
  cleanupButton: document.querySelector('#cleanupButton'),
  copyButton: document.querySelector('#copyButton'),
  detectQqButton: document.querySelector('#detectQqButton'),
  inlineMessage: document.querySelector('#inlineMessage'),
  proxyPort: document.querySelector('#proxyPort'),
  qqNumber: document.querySelector('#qqNumber'),
  profileAvatar: document.querySelector('#profileAvatar'),
  profileFallback: document.querySelector('#profileFallback'),
  profileGid: document.querySelector('#profileGid'),
  profileIdentity: document.querySelector('#profileIdentity'),
  profileName: document.querySelector('#profileName'),
  profileOpenId: document.querySelector('#profileOpenId'),
  profileState: document.querySelector('#profileState'),
  saveButton: document.querySelector('#saveButton'),
  serverToken: document.querySelector('#serverToken'),
  serverUrl: document.querySelector('#serverUrl'),
  stageList: [...document.querySelectorAll('#stageList li')],
  startButton: document.querySelector('#startButton'),
  startButtonText: document.querySelector('#startButtonText'),
  statusCode: document.querySelector('#statusCode'),
  statusDetail: document.querySelector('#statusDetail'),
  statusSignal: document.querySelector('#statusSignal'),
  statusTitle: document.querySelector('#statusTitle'),
  stopButton: document.querySelector('#stopButton'),
  testButton: document.querySelector('#testButton'),
  toggleToken: document.querySelector('#toggleToken'),
  tokenHint: document.querySelector('#tokenHint'),
}

const stageOrder = ['preparing_proxy', 'waiting_login', 'code_captured', 'syncing']
const activePhases = new Set(['preparing_proxy', 'waiting_login', 'code_captured', 'syncing'])

function settingsPayload() {
  return {
    settings: {
      server_url: elements.serverUrl.value.trim(),
      account_name: elements.accountName.value.trim(),
      qq_number: elements.qqNumber.value.trim(),
      auto_sync: elements.autoSync.checked,
      proxy_port: Number(elements.proxyPort.value),
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

function showMessage(message, isError = false) {
  elements.inlineMessage.textContent = message || ''
  elements.inlineMessage.classList.toggle('error', isError)
}

function fillSettings(data) {
  elements.serverUrl.value = data.settings.server_url || ''
  elements.accountName.value = data.settings.account_name || 'Windows QQ'
  elements.qqNumber.value = data.settings.qq_number || ''
  elements.autoSync.checked = data.settings.auto_sync !== false
  elements.proxyPort.value = data.settings.proxy_port || 8899
  elements.tokenHint.textContent = data.token_configured
    ? 'Token 已安全保存在 Windows 凭据管理器；留空即保留。'
    : 'Token 保存在 Windows 凭据管理器，不写入配置文件。'
}

function renderProfile(profile) {
  if (!profile)
    return
  elements.profileState.textContent = profile.running ? '运行中' : '已同步'
  elements.profileState.classList.add('ready')
  elements.profileName.textContent = profile.nickname || profile.accountName || `远程账号 #${profile.accountId}`
  elements.profileIdentity.textContent = profile.qqNumber
    ? `QQ ${profile.qqNumber}`
    : `远程账号 #${profile.accountId}`
  elements.profileGid.textContent = profile.gid || '等待登录回填'
  elements.profileOpenId.textContent = profile.openId
    ? `${profile.openId.slice(0, 8)}${profile.openId.length > 8 ? '…' : ''}`
    : '等待登录回填'

  setProfileAvatar(profile.avatarUrl)
}

function setProfileAvatar(avatarUrl) {
  if (!avatarUrl)
    return
  elements.profileAvatar.src = avatarUrl
  elements.profileAvatar.classList.add('visible')
  elements.profileFallback.classList.add('hidden')
}

function renderLocalIdentity(identity) {
  elements.profileState.textContent = '本机已识别'
  elements.profileState.classList.add('ready')
  elements.profileName.textContent = identity.nickname || 'Windows QQ'
  elements.profileIdentity.textContent = `QQ ${identity.qqNumber}`
  elements.profileGid.textContent = '同步后回填'
  elements.profileOpenId.textContent = '同步后回填'
  setProfileAvatar(identity.avatarUrl)
}

function updateStages(phase) {
  const activeIndex = stageOrder.indexOf(phase)
  elements.stageList.forEach((item) => {
    const index = stageOrder.indexOf(item.dataset.stage)
    item.classList.toggle('active', index === activeIndex)
    item.classList.toggle('done', activeIndex > index || phase === 'completed')
  })
}

function renderStatus(status) {
  const phase = status.phase || 'idle'
  const isActive = activePhases.has(phase)
  elements.statusTitle.textContent = status.title
  elements.statusDetail.textContent = status.detail
  elements.statusCode.textContent = phase.replaceAll('_', ' ').toUpperCase()
  elements.statusSignal.className = `status-signal ${isActive ? 'active' : ''} ${phase === 'completed' ? 'success' : ''} ${phase === 'error' ? 'error' : ''}`
  elements.startButton.disabled = isActive
  elements.startButtonText.textContent = isActive ? '正在获取…' : '启动获取'
  elements.stopButton.disabled = !isActive
  elements.copyButton.classList.toggle('hidden', !status.code_available)
  updateStages(phase)
  renderProfile(status.profile)
}

async function saveSettings() {
  setBusy(elements.saveButton, true, '保存中…')
  showMessage('')
  try {
    const result = await invoke('save_settings', settingsPayload())
    fillSettings(result)
    elements.serverToken.value = ''
    showMessage('设置已保存。')
  }
  catch (error) {
    showMessage(String(error), true)
  }
  finally {
    setBusy(elements.saveButton, false)
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
    showMessage(`连接成功：${result.username} (${result.role})`)
  }
  catch (error) {
    showMessage(String(error), true)
  }
  finally {
    setBusy(elements.testButton, false)
  }
}

async function startCapture() {
  showMessage('')
  try {
    await detectLocalQq({ overwrite: true, notify: false })
    await invoke('save_settings', settingsPayload())
    elements.serverToken.value = ''
    await invoke('start_capture')
  }
  catch (error) {
    showMessage(String(error), true)
  }
}

async function stopCapture() {
  try {
    await invoke('stop_capture')
  }
  catch (error) {
    showMessage(String(error), true)
  }
}

async function cleanupNetwork() {
  setBusy(elements.cleanupButton, true, '清理中…')
  try {
    await invoke('cleanup_network')
    showMessage('系统代理与临时证书已清理。')
  }
  catch (error) {
    showMessage(String(error), true)
  }
  finally {
    setBusy(elements.cleanupButton, false)
  }
}

async function copyCode() {
  try {
    const code = await invoke('get_captured_code')
    await navigator.clipboard.writeText(code)
    showMessage('Code 已复制，请尽快使用。')
  }
  catch (error) {
    showMessage(String(error), true)
  }
}

async function detectLocalQq({ overwrite = true, notify = true, render = true } = {}) {
  if (notify)
    setBusy(elements.detectQqButton, true, '检测中…')
  try {
    const identity = await invoke('detect_local_qq')
    if (overwrite || !elements.qqNumber.value.trim())
      elements.qqNumber.value = identity.qqNumber
    if (render)
      renderLocalIdentity(identity)
    if (notify)
      showMessage(`已识别当前 Windows QQ：${identity.nickname || identity.qqNumber}`)
    return identity
  }
  catch (error) {
    if (notify)
      showMessage(String(error), true)
    return null
  }
  finally {
    if (notify)
      setBusy(elements.detectQqButton, false)
  }
}

async function bootstrap() {
  window.scrollTo(0, 0)
  const data = await invoke('get_bootstrap')
  fillSettings(data)
  renderStatus(data.status)
  await detectLocalQq({ overwrite: false, notify: false, render: !data.status.profile })
  if (data.startup_warning)
    showMessage(data.startup_warning, true)
  await listen('capture-status', event => renderStatus(event.payload))
}

elements.saveButton.addEventListener('click', saveSettings)
elements.testButton.addEventListener('click', testConnection)
elements.startButton.addEventListener('click', startCapture)
elements.stopButton.addEventListener('click', stopCapture)
elements.cleanupButton.addEventListener('click', cleanupNetwork)
elements.copyButton.addEventListener('click', copyCode)
elements.detectQqButton.addEventListener('click', () => detectLocalQq())
elements.toggleToken.addEventListener('click', () => {
  const hidden = elements.serverToken.type === 'password'
  elements.serverToken.type = hidden ? 'text' : 'password'
  elements.toggleToken.textContent = hidden ? '隐藏' : '显示'
})
elements.qqNumber.addEventListener('input', () => {
  elements.qqNumber.value = elements.qqNumber.value.replace(/\D/g, '').slice(0, 12)
})
elements.profileAvatar.addEventListener('error', () => {
  elements.profileAvatar.classList.remove('visible')
  elements.profileFallback.classList.remove('hidden')
})

bootstrap().catch(error => showMessage(String(error), true))
