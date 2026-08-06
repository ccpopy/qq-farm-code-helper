const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const elements = {
  accountName: document.querySelector('#accountName'),
  autoSync: document.querySelector('#autoSync'),
  cleanupButton: document.querySelector('#cleanupButton'),
  copyButton: document.querySelector('#copyButton'),
  inlineMessage: document.querySelector('#inlineMessage'),
  proxyPort: document.querySelector('#proxyPort'),
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
  elements.autoSync.checked = data.settings.auto_sync !== false
  elements.proxyPort.value = data.settings.proxy_port || 8899
  elements.tokenHint.textContent = data.token_configured
    ? 'Token 已安全保存在 Windows 凭据管理器；留空即保留。'
    : 'Token 保存在 Windows 凭据管理器，不写入配置文件。'
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

async function bootstrap() {
  const data = await invoke('get_bootstrap')
  fillSettings(data)
  renderStatus(data.status)
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
elements.toggleToken.addEventListener('click', () => {
  const hidden = elements.serverToken.type === 'password'
  elements.serverToken.type = hidden ? 'text' : 'password'
  elements.toggleToken.textContent = hidden ? '隐藏' : '显示'
})

bootstrap().catch(error => showMessage(String(error), true))
