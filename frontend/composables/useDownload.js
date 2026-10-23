import { ref, computed, onBeforeUnmount } from 'vue'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'
import { useJobPolling } from './useJobPolling.js'
import { useNotifications } from './useNotifications.js'
import { useAppStore } from '../stores/app.js'

/**
 * Localize progress stage text to Chinese
 */
export function localizeProgressStage(stage) {
  const raw = String(stage ?? '').trim()
  if (!raw) return '准备中…'
  if (/[\u4e00-\u9fff]/.test(raw)) return raw

  const normalized = raw.toLowerCase().replace(/[_\s]+/g, '-')
  const exactMap = {
    auth: '获取下载信息',
    authenticate: '获取下载信息',
    authentication: '获取下载信息',
    metadata: '读取元数据',
    'fetch-metadata': '读取元数据',
    'download-info': '获取下载信息',
    'download-start': '开始下载',
    'start-download': '开始下载',
    'download-progress': '下载中',
    downloading: '下载中',
    download: '下载中',
    saving: '保存文件',
    save: '保存文件',
    merge: '合并分块',
    merging: '合并分块',
    sign: '写入签名',
    signing: '写入签名',
    extract: '提取信息',
    extracting: '提取信息',
    inspect: '校验 IPA',
    inspecting: '校验 IPA',
    verify: '校验 IPA',
    verifying: '校验 IPA',
    processing: '处理中',
    process: '处理中',
    prepare: '准备中…',
    preparing: '准备中…',
    queued: '排队中',
    queue: '排队中',
    waiting: '等待中',
    done: '下载已完成',
    ready: '下载已完成',
    completed: '下载已完成',
    complete: '下载已完成',
    failed: '下载失败',
    failure: '下载失败',
    error: '下载失败'
  }

  if (exactMap[normalized]) return exactMap[normalized]
  if (normalized.includes('auth') || normalized.includes('credential') || normalized.includes('license')) return '获取下载信息'
  if (normalized.includes('metadata') || normalized.includes('info')) return '读取元数据'
  if (normalized.includes('download') && normalized.includes('start')) return '开始下载'
  if (normalized.includes('download') && normalized.includes('progress')) return '下载中'
  if (normalized.includes('download') || normalized.includes('transfer')) return '下载中'
  if (normalized.includes('save') || normalized.includes('write-file')) return '保存文件'
  if (normalized.includes('merge')) return '合并分块'
  if (normalized.includes('sign')) return '写入签名'
  if (normalized.includes('extract')) return '提取信息'
  if (normalized.includes('inspect') || normalized.includes('verify')) return '校验 IPA'
  if (normalized.includes('process')) return '处理中'
  if (normalized.includes('queue')) return '排队中'
  if (normalized.includes('wait')) return '等待中'
  if (normalized.includes('prepare')) return '准备中…'
  if (normalized.includes('ready') || normalized.includes('done') || normalized.includes('complete')) return '下载已完成'
  if (normalized.includes('fail') || normalized.includes('error')) return '下载失败'

  return '处理中'
}

function normalizeDownloadedIpaFileForQueue(file) {
  if (!file) return null
  return {
    id: file.id ?? file.recordId ?? file.record_id ?? '',
    recordId: file.recordId ?? file.record_id ?? file.id ?? null,
    appId: String(file.appId ?? file.app_id ?? file.trackId ?? ''),
    appName: String(file.appName ?? file.app_name ?? file.name ?? file.fileName ?? file.file_name ?? ''),
    accountEmail: String(file.accountEmail ?? file.account_email ?? file.apple_id ?? file.email ?? ''),
    version: String(file.version ?? file.bundle_version ?? file.appVersion ?? ''),
    versionId: String(file.versionId ?? file.version_id ?? file.appVerId ?? file.app_version_id ?? file.external_identifier ?? ''),
    artworkUrl: file.artworkUrl ?? file.artwork_url ?? file.icon_url ?? '',
    artistName: file.artistName ?? file.artist_name ?? '',
    downloadUrl: file.downloadUrl ?? file.download_url ?? '',
    installUrl: file.installUrl ?? file.install_url ?? '',
    fileSize: file.fileSize ?? file.file_size ?? 0,
    packageKind: file.packageKind ?? file.package_kind ?? '',
    otaInstallable: Boolean(file.otaInstallable ?? file.ota_installable),
    installMethod: file.installMethod ?? file.install_method ?? '',
    inspection: file.inspection ?? null
  }
}

/**
 * Download management composable
 * Handles SSE connections, polling, purchase checks, and download state management
 * @param {Object} options - Configuration options
 * @param {Function} options.onDownloadStarted - Callback when download starts
 * @param {Function} options.onDownloadComplete - Callback when download completes
 * @param {Function} options.onDownloadFailed - Callback when download fails
 * @param {Object} options.selectedApp - Currently selected app
 * @param {Object} options.selectedAccount - Currently selected account
 * @returns {Object} Download state and methods
 */
export function useDownload(options = {}) {
  const {
    onDownloadStarted,
    onDownloadComplete,
    onDownloadFailed,
    selectedApp = ref(null),
    selectedAccount = ref(null),
    accounts = ref([])
  } = options

  const appStore = useAppStore()
  const notifications = useNotifications()

  // ===== Download State =====
  const downloading = ref(false)
  const showProgress = ref(false)
  const progressPercent = ref(0)
  const progressStage = ref('等待任务…')
  const logs = ref('')

  // Download result state
  const downloadReadyUrl = ref('')
  const downloadReadyFileSize = ref(0)
  const downloadInstallUrl = ref('')
  const downloadPackageKind = ref('')
  const downloadOtaInstallable = ref(false)
  const downloadInstallMethod = ref('')
  const downloadInspection = ref(null)
  const showActionButtons = ref(false)

  // Active download tracking
  const activeDownloadAppId = ref('')
  const activeDownloadVersionId = ref('')
  const activeDownloadAccountEmail = ref('')

  // Purchase state
  const checkingPurchaseStatus = ref(false)
  const claimingSelectedApp = ref(false)
  const purchaseStatus = ref({ purchased: null, needsPurchase: false, status: 'unknown', error: null })
  const purchaseStatusText = ref('待检测')

  // SSE connection tracking
  let activeEventSource = null

  // ===== Computed Properties =====
  const purchaseRequired = computed(() => !!purchaseStatus.value.needsPurchase)

  const claimRequired = computed(() => {
    if (!purchaseRequired.value) return false
    const price = getSelectedAppPrice()
    return price !== null && price <= 0
  })

  const downloadBlocked = computed(() => checkingPurchaseStatus.value || purchaseRequired.value)

  const isStoreMismatchError = (msg) => {
    if (!msg || typeof msg !== 'string') return false
    return /account not in this store/i.test(msg) ||
      /not valid for use in the .* store/i.test(msg)
  }

  const getStoreMismatchMessage = (account) => {
    const reg = account?.region ? `（如 ${account.region}）` : ''
    return `当前选中的账号区域${reg}与该应用所在商店不一致，请切换为对应区域账号`
  }

  const downloadBlockedReason = computed(() => {
    if (checkingPurchaseStatus.value) return '正在检测购买状态…'
    const accountIndex = selectedAccount.value
    const account = accountIndex !== null && accountIndex !== undefined ? accounts.value[accountIndex] : null
    const err = purchaseStatus.value?.error || ''
    if (isStoreMismatchError(err)) {
      return getStoreMismatchMessage(account)
    }
    if (err) return `检测失败：${err}`
    if (!purchaseRequired.value) return ''
    const price = getSelectedAppPrice()
    if (price !== null && price > 0) return '当前账号未购买：请先在App Store购买应用'
    return '当前账号未领取：请先点击"获取应用"后再下载'
  })

  const purchaseActionLabel = computed(() => {
    if (claimRequired.value) return '获取应用'
    const price = getSelectedAppPrice()
    if (price !== null && price > 0) return '前往 App Store 购买'
    return '获取应用'
  })

  const isHttps = ref(false)
  const currentProtocol = ref('')

  // ===== Helper Functions =====
  const addLog = (message) => {
    const timestamp = new Date().toLocaleTimeString()
    logs.value += `[${timestamp}] ${message}\n`
  }

  const getSelectedAppPrice = () => {
    const price = Number(selectedApp.value?.price)
    return Number.isFinite(price) ? price : null
  }

  const formatFileSize = (bytes) => {
    if (!bytes) return ''
    const units = ['B', 'KB', 'MB', 'GB']
    let value = Number(bytes)
    if (!Number.isFinite(value) || value <= 0) return ''
    let unitIndex = 0
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024
      unitIndex += 1
    }
    return `${value.toFixed(value >= 100 || unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
  }

  const openExternalUrl = (url) => {
    if (!url) return false
    const popup = window.open(url, '_blank', 'noopener,noreferrer')
    if (popup) return true
    window.location.href = url
    return true
  }

  // ===== Purchase Status Functions =====
  let purchaseRequestId = 0

  const refreshPurchaseStatus = async (appId, appVerId) => {
    const requestId = ++purchaseRequestId
    const selectedAppId = selectedApp.value?.trackId
    const targetAppId = appId || selectedAppId
    const accountIndex = selectedAccount.value
    const account = accounts.value[accountIndex]
    const token = account?.token
    const isCurrent = () => requestId === purchaseRequestId &&
      selectedApp.value?.trackId === selectedAppId &&
      selectedAccount.value === accountIndex && accounts.value[accountIndex]?.token === token

    // Never retain an ownership verdict from the previous target or a failed check.
    purchaseStatus.value = { purchased: null, needsPurchase: false, status: 'unknown', error: null }
    checkingPurchaseStatus.value = false
    if (!targetAppId) {
      purchaseStatusText.value = '待检测'
      return false
    }
    if (accountIndex === null || accountIndex === undefined || accountIndex === '') {
      purchaseStatusText.value = '请选择账号后检测'
      return false
    }
    if (!token) {
      purchaseStatusText.value = '账号无效'
      return false
    }

    checkingPurchaseStatus.value = true
    purchaseStatusText.value = '正在检测购买状态…'
    try {
      const query = `token=${encodeURIComponent(token)}&appid=${encodeURIComponent(targetAppId)}`
      const versionParam = appVerId ? `&appVerId=${encodeURIComponent(appVerId)}` : ''
      const { response, data } = await apiFetch(`${API_BASE}/purchase-status?${query}${versionParam}`)
      if (!isCurrent()) return false
      if (response?.ok === false || !data?.ok) throw new Error(data?.error || '检测购买状态失败')
      const payload = data?.data || {}
      const error = payload.error || (payload.status === 'error' ? '检测购买状态失败' : null)
      purchaseStatus.value = {
        purchased: error ? null : !!payload.purchased,
        needsPurchase: !error && !!payload.needsPurchase,
        status: error ? 'error' : payload.status || 'unknown',
        error
      }

      const price = getSelectedAppPrice()
      if (isStoreMismatchError(error)) {
        purchaseStatusText.value = getStoreMismatchMessage(account)
      } else if (error) {
        purchaseStatusText.value = `检测失败：${error}`
      } else if (payload.purchased) {
        purchaseStatusText.value = price !== null && price > 0 ? '当前账号已购买' : '当前账号已领取'
        return true
      } else if (payload.needsPurchase) {
        purchaseStatusText.value = price !== null && price > 0 ? '当前账号未购买' : '当前账号未领取'
      } else {
        purchaseStatusText.value = '状态未知'
      }
      return false
    } catch (error) {
      if (!isCurrent()) return false
      const message = error?.message || '检测购买状态失败'
      purchaseStatus.value = { purchased: null, needsPurchase: false, status: 'error', error: message }
      purchaseStatusText.value = isStoreMismatchError(message)
        ? getStoreMismatchMessage(account) : `检测失败：${message}`
      console.warn('Failed to refresh purchase status:', error)
      return false
    } finally {
      if (requestId === purchaseRequestId) checkingPurchaseStatus.value = false
    }
  }

  const preflightPurchaseGate = async (account, appId, appVerId) => {
    if (!account?.token) throw new Error('账号无效，请重新选择账号')
    return refreshPurchaseStatus(appId, appVerId)
  }

  // ===== Download Functions =====
  const startDownloadWithProgress = async (
    appId,
    appVerId,
    autoPurchase = false,
    autoInstallRequested = false,
    versionLabel = ''
  ) => {
    const accountIndex = selectedAccount.value
    if (accountIndex === null || accountIndex === undefined) {
      throw new Error('请选择登录账号')
    }
    if (!appId) {
      throw new Error('请填写 APPID')
    }

    const account = accounts.value[accountIndex]
    if (!account) {
      throw new Error('当前账号不存在，请重新选择账号')
    }

    if (!autoPurchase) {
      const selectedAppId = selectedApp.value?.trackId
      const token = account.token
      const allowed = await preflightPurchaseGate(account, appId, appVerId)
      if (selectedApp.value?.trackId !== selectedAppId || selectedAccount.value !== accountIndex ||
          accounts.value[accountIndex]?.token !== token) {
        throw new Error('应用或账号已切换，请重新下载')
      }
      if (!allowed) {
        if (!purchaseRequired.value) {
          throw new Error(downloadBlockedReason.value || purchaseStatusText.value || '状态未知，请重试')
        }
        const err = purchaseStatus.value?.error || ''
        if (isStoreMismatchError(err)) {
          const msg = getStoreMismatchMessage(account)
          addLog(`[拦截] ${msg}`)
          throw new Error(msg)
        }
        const price = getSelectedAppPrice()
        if (price !== null && price > 0) {
          addLog('[拦截] 当前账号未购买此应用，请先前往 App Store 购买后再下载')
        } else if (price !== null && price <= 0) {
          addLog('[拦截] 当前账号未领取此免费应用，请先点击"获取应用"后再下载')
        }
        const downloadBlocked = downloadBlockedReason.value ||
          (price !== null && price > 0 ? '请先在 App Store 购买此应用' :
           price !== null && price <= 0 ? '请先获取免费应用' : '当前账号未获取此应用权限')
        throw new Error(downloadBlocked)
      }
    }

    // Reset progress
    downloading.value = true
    showProgress.value = true
    progressPercent.value = 0
    progressStage.value = '准备中…'
    logs.value = ''
    downloadReadyUrl.value = ''
    downloadReadyFileSize.value = 0
    downloadInstallUrl.value = ''
    downloadPackageKind.value = ''
    downloadOtaInstallable.value = false
    downloadInstallMethod.value = ''
    downloadInspection.value = null
    showActionButtons.value = false
    activeDownloadAppId.value = String(appId || '')
    activeDownloadVersionId.value = String(appVerId || '')
    activeDownloadAccountEmail.value = String(account.email || '')
    addLog('[进度] 创建下载任务…')

    addLog(`[进度] 使用账号 ${account.email} 发起任务，token=${String(account.token).slice(0, 8)}…`)

    try {
      const resolvedVersionLabel = String(versionLabel || selectedApp.value?.version || '')
      const { data } = await apiFetch(`${API_BASE}/start-download-direct`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({
          token: account.token,
          appid: appId,
          appVerId: appVerId ? String(appVerId) : undefined,
          autoPurchase: !!autoPurchase,
          appName: selectedApp.value?.trackName || undefined,
          bundleId: selectedApp.value?.bundleId || undefined,
          appVersion: resolvedVersionLabel || undefined,
          artworkUrl: selectedApp.value?.artworkUrl100 || selectedApp.value?.artworkUrl60 || undefined,
          artistName: selectedApp.value?.artistName || undefined
        })
      })

      if (!data.ok) {
        const errMsg = data.error || ''
        if (isStoreMismatchError(errMsg)) {
          downloading.value = false
          showProgress.value = false
          activeDownloadAppId.value = ''
          activeDownloadVersionId.value = ''
          activeDownloadAccountEmail.value = ''
          throw new Error(getStoreMismatchMessage(account))
        }
        if (data.needsPurchase && !autoPurchase) {
          downloading.value = false
          showProgress.value = false
          activeDownloadAppId.value = ''
          activeDownloadVersionId.value = ''
          activeDownloadAccountEmail.value = ''
          throw new Error(errMsg || downloadBlockedReason.value || '当前账号未购买/未领取')
        }
        throw new Error(data.error || '未知错误')
      }

      const taskPayload = data?.data || {}
      const jobId = String(taskPayload?.jobId ?? data?.jobId ?? '').trim()
      if (!jobId) {
        throw new Error('下载任务创建成功，但返回中缺少 jobId')
      }
      addLog(`[进度] 任务已创建：${jobId}`)

      // Create queue item
      const queueItem = {
        id: jobId,
        appId: String(appId || selectedApp.value.trackId || ''),
        versionId: String(appVerId || ''),
        version: resolvedVersionLabel,
        artworkUrl: selectedApp.value?.artworkUrl100 || selectedApp.value?.artworkUrl60 || '',
        appName: selectedApp.value?.trackName || appId,
        artistName: selectedApp.value?.artistName || '',
        app: selectedApp.value,
        account: account,
        accountEmail: account.email || '',
        status: 'downloading',
        progress: 0,
        stage: '准备中…',
        autoInstallRequested,
        logs: logs.value,
        timestamp: new Date().toISOString(),
        updatedAt: new Date().toISOString()
      }

      if (taskPayload.recordId) {
        queueItem.recordId = taskPayload.recordId
      }
      if (taskPayload.reused != null) {
        queueItem.reused = !!taskPayload.reused
      }
      if (taskPayload.downloadUrl) {
        queueItem.downloadUrl = taskPayload.downloadUrl
      }
      if (taskPayload.installUrl) {
        queueItem.installUrl = taskPayload.installUrl
      }
      if (taskPayload.fileSize) {
        queueItem.fileSize = taskPayload.fileSize
      }

      if (onDownloadStarted) {
        onDownloadStarted(queueItem)
      }

      // Connect to SSE / fallback polling
      connectToSSE(jobId, queueItem)
    } catch (error) {
      downloading.value = false
      showProgress.value = false
      activeDownloadAppId.value = ''
      activeDownloadVersionId.value = ''
      activeDownloadAccountEmail.value = ''
      addLog(`[进度] 创建任务失败：${error.message}`)
      throw error
    }
  }

  // ===== SSE Connection =====
  const connectToSSE = (jobId, queueItem) => {
    let es
    try {
      const origin = window.location.origin || `${window.location.protocol}//${window.location.host}`
      const sseUrl = new URL(`${API_BASE}/progress-sse?jobId=${encodeURIComponent(jobId)}`, origin).toString()
      es = new EventSource(sseUrl)
      activeEventSource = es
    } catch (error) {
      addLog(`[进度] SSE 初始化失败：${error.message}`)
      pollJobStatus(jobId, queueItem)
      return
    }

    es.addEventListener('progress', (ev) => {
      try {
        const data = JSON.parse(ev.data)

        if (data?.progress?.percent != null) {
          progressPercent.value = data.progress.percent
          appStore.updateQueueItem(jobId, { progress: data.progress.percent })
        }

        if (data?.progress?.stage) {
          progressStage.value = localizeProgressStage(data.progress.stage)
          appStore.updateQueueItem(jobId, { stage: progressStage.value })
        }

        if (data?.error) {
          addLog(`[错误] ${data.error}`)
          const appName = selectedApp.value?.trackName || activeDownloadAppId.value
          notifications.notifyDownloadFailed(appName, data.error)
          appStore.updateQueueItem(jobId, {
            status: 'failed',
            error: data.error
          })
        }

        if (data.status === 'ready') {
          progressPercent.value = 100
          progressStage.value = '下载已完成'
          addLog('[进度] 文件已保存到服务器，可在任务完成后刷新获取交付信息')

          appStore.updateQueueItem(jobId, {
            status: 'completed',
            progress: 100
          })
        }
      } catch (e) {
        console.error(e)
      }
    })

    es.addEventListener('log', (ev) => {
      try {
        const { line } = JSON.parse(ev.data)
        if (line) {
          addLog(line)
          appStore.updateQueueItem(jobId, { logs: logs.value })
        }
      } catch {}
    })

    es.addEventListener('end', (ev) => {
      try {
        const data = JSON.parse(ev.data || '{}')
        if (data.status === 'ready') {
          handleDownloadComplete(jobId, queueItem)
        } else if (data.status === 'failed') {
          handleDownloadFailed(jobId, queueItem, data.error)
        } else {
          addLog(`[结束] 任务结束：${data.status || 'unknown'}`)
        }
      } catch {}
      es.close()
      activeEventSource = null
    })

    es.onerror = () => {
      addLog('[错误] SSE 连接断开，切换为轮询模式')
      es.close()
      activeEventSource = null
      pollJobStatus(jobId, queueItem)
    }
  }

  // ===== Polling Fallback =====
  const pollJobStatus = (jobId, queueItem) => {
    addLog('[进度] SSE 不可用，自动切换为轮询模式')

    const { startPolling } = useJobPolling({
      pollInterval: 1500,
      maxFailures: 5,
      isFinalStatus: (status) => ['completed', 'ready', 'failed', 'error'].includes(status),
      onUpdate: (taskId, snapshot) => {
        if (snapshot.progress != null) {
          progressPercent.value = snapshot.progress
          appStore.updateQueueItem(jobId, { progress: snapshot.progress })
        }

        if (snapshot.stage) {
          progressStage.value = localizeProgressStage(snapshot.stage)
          appStore.updateQueueItem(jobId, { stage: progressStage.value })
        }

        if (snapshot.error) {
          addLog(`[错误] ${snapshot.error}`)
        }
      },
      onComplete: (taskId, snapshot) => {
        handleDownloadComplete(taskId, queueItem, snapshot)
      },
      onFailed: (taskId, snapshot) => {
        handleDownloadFailed(taskId, queueItem, snapshot.error || '任务失败')
      }
    })

    startPolling(jobId)
  }

  // ===== Download Handlers =====
  const handleDownloadComplete = async (jobId, queueItem, snapshot = null) => {
    downloading.value = false
    progressPercent.value = 100
    progressStage.value = '下载已完成'

    if (snapshot?.installMethod === 'download_only') {
      addLog('[进度] 文件已保存到服务器，仅支持下载导出')
    } else {
      addLog('[进度] 文件已保存到服务器，可手动下载或安装')
    }

    // Get job info
    const jobInfo = snapshot || await fetchJobInfo(jobId)
    if (!jobInfo) {
      addLog('[错误] 获取任务信息失败')
      return
    }

    const jobData = jobInfo

    appStore.updateQueueItem(jobId, {
      status: 'completed',
      progress: 100,
      downloadUrl: jobData.downloadUrl,
      installUrl: jobData.installUrl,
      fileSize: jobData.fileSize || 0,
      packageKind: jobData.packageKind,
      otaInstallable: jobData.otaInstallable,
      installMethod: jobData.installMethod,
      inspection: jobData.inspection
    })

    downloadReadyUrl.value = jobData.downloadUrl || ''
    downloadReadyFileSize.value = jobData.fileSize || 0
    downloadInstallUrl.value = jobData.installUrl || ''
    downloadPackageKind.value = jobData.packageKind || ''
    downloadOtaInstallable.value = !!jobData.otaInstallable
    downloadInstallMethod.value = jobData.installMethod || ''
    downloadInspection.value = jobData.inspection || null
    showActionButtons.value = !!(jobData.downloadUrl || jobData.installUrl)

    const refreshedFile = await refreshDownloadedFileForTask(jobId, queueItem, jobData)
    if (refreshedFile) {
      if (!downloadReadyUrl.value && refreshedFile.downloadUrl) downloadReadyUrl.value = refreshedFile.downloadUrl
      if (!downloadInstallUrl.value && refreshedFile.installUrl) downloadInstallUrl.value = refreshedFile.installUrl
      if (!downloadReadyFileSize.value && refreshedFile.fileSize) downloadReadyFileSize.value = refreshedFile.fileSize
      if (!downloadPackageKind.value && refreshedFile.packageKind) downloadPackageKind.value = refreshedFile.packageKind
      if (!downloadInspection.value && refreshedFile.inspection) downloadInspection.value = refreshedFile.inspection
      downloadOtaInstallable.value = downloadOtaInstallable.value || !!refreshedFile.otaInstallable
      if (!downloadInstallMethod.value && refreshedFile.installMethod) downloadInstallMethod.value = refreshedFile.installMethod
      showActionButtons.value = !!(downloadReadyUrl.value || downloadInstallUrl.value)
    }

    addLog('[完成] 任务已就绪')

    if (queueItem?.autoInstallRequested) {
      maybeAutoInstallAfterReady({ installUrl: jobData.installUrl, downloadUrl: jobData.downloadUrl })
    }

    if (onDownloadComplete) {
      onDownloadComplete(jobData)
    }

    const appName = selectedApp.value?.trackName || activeDownloadAppId.value
    notifications.notifyDownloadComplete(appName)

    // 保留完成任务在队列中，详情页可继续用队列项回显下载/安装入口。
  }

  const refreshDownloadedFileForTask = async (jobId, queueItem, jobData = {}) => {
    try {
      const { data } = await apiFetch(`${API_BASE}/ipa-files`, { credentials: 'include' })
      if (!data?.ok || !Array.isArray(data.data)) return null

      const appId = String(queueItem?.appId || activeDownloadAppId.value || '')
      const versionId = String(queueItem?.versionId || activeDownloadVersionId.value || '')
      const accountEmail = String(queueItem?.accountEmail || activeDownloadAccountEmail.value || '')

      const match = data.data
        .map(normalizeDownloadedIpaFileForQueue)
        .filter(Boolean)
        .find((file) => {
          if (appId && file.appId !== appId) return false
          if (accountEmail && file.accountEmail && file.accountEmail !== accountEmail) return false
          if (versionId && file.versionId && file.versionId !== versionId) return false
          return true
        })

      if (!match) return null
      appStore.updateQueueItem(jobId, {
        recordId: match.recordId,
        appName: match.appName || queueItem?.appName,
        version: match.version || queueItem?.version,
        versionId: match.versionId || queueItem?.versionId,
        artworkUrl: match.artworkUrl || queueItem?.artworkUrl,
        artistName: match.artistName || queueItem?.artistName,
        accountEmail: match.accountEmail || queueItem?.accountEmail,
        downloadUrl: match.downloadUrl || jobData.downloadUrl,
        installUrl: match.installUrl || jobData.installUrl,
        fileSize: match.fileSize || jobData.fileSize || 0,
        packageKind: match.packageKind || jobData.packageKind,
        otaInstallable: match.otaInstallable || jobData.otaInstallable,
        installMethod: match.installMethod || jobData.installMethod,
        inspection: match.inspection || jobData.inspection,
        status: 'completed',
        progress: 100,
        stage: '下载已完成'
      })
      return match
    } catch (error) {
      console.warn('Failed to refresh downloaded files after completion:', error)
      return null
    }
  }

  const handleDownloadFailed = (jobId, queueItem, error) => {
    downloading.value = false
    showProgress.value = false
    showActionButtons.value = false
    activeDownloadAppId.value = ''
    activeDownloadVersionId.value = ''
    activeDownloadAccountEmail.value = ''
    addLog(`[失败] ${error || '任务失败'}`)

    appStore.updateQueueItem(jobId, {
      status: 'failed',
      error: error || '任务失败'
    })

    if (queueItem) {
      queueItem.status = 'error'
      queueItem.error = error || '任务失败'
    }

    if (onDownloadFailed) {
      onDownloadFailed(error)
    }

    const appName = selectedApp.value?.trackName || activeDownloadAppId.value
    notifications.notifyDownloadFailed(appName, error)
  }

  const fetchJobInfo = async (jobId) => {
    try {
      const { data } = await apiFetch(`${API_BASE}/job-info?jobId=${encodeURIComponent(jobId)}`)
      if (data.ok && data.data) {
        return data.data
      }
    } catch (error) {
      console.warn('Failed to fetch job info:', error)
    }
    return null
  }

  const maybeAutoInstallAfterReady = async ({ installUrl = '' } = {}) => {
    if (!installUrl) {
      addLog('[安装] 当前包未生成可用安装链接，请在任务完成后改用下载按钮导出 IPA')
      return
    }

    const isHttpsEnvironment = window.location.protocol === 'https:'
    const isLocalhost = window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1'

    if (isHttpsEnvironment) {
      openInstallUrl(installUrl)
    } else if (isLocalhost) {
      // Allow user to confirm for localhost
      console.log('[安装] localhost 环境下 OTA 安装可能失败')
    }
  }

  const openInstallUrl = (url) => {
    if (!url) return
    window.location.assign(url)
  }

  // ===== Claim/Purchase Functions =====
  const claimSelectedAppInBackground = async (appId, appVerId) => {
    const accountIndex = selectedAccount.value
    if (accountIndex === null || accountIndex === undefined) {
      throw new Error('请选择登录账号')
    }

    const account = accounts.value[accountIndex]
    if (!account) {
      throw new Error('当前账号不存在，请重新选择账号')
    }

    if (!selectedApp.value?.trackId) {
      throw new Error('当前应用信息无效')
    }

    claimingSelectedApp.value = true
    purchaseStatusText.value = '正在后台获取应用…'

    try {
      const { response, data } = await apiFetch(`${API_BASE}/claim`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({
          token: account.token,
          appid: appId || String(selectedApp.value.trackId),
          appVerId: appVerId || undefined
        })
      })

      if (!response.ok || !data?.ok) {
        const errMsg = data?.error || ''
        if (isStoreMismatchError(errMsg)) {
          throw new Error(getStoreMismatchMessage(account))
        }
        throw new Error(errMsg || '获取应用失败')
      }

      const verified = await refreshPurchaseStatus(appId, appVerId)
      if (!verified) {
        throw new Error(downloadBlockedReason.value || purchaseStatusText.value || '获取应用后状态仍未更新，请稍后重试')
      }

      claimingSelectedApp.value = false
    } catch (error) {
      claimingSelectedApp.value = false
      throw error
    }
  }

  // ===== Utility Functions =====
  const resetDownloadState = () => {
    downloading.value = false
    showProgress.value = false
    progressPercent.value = 0
    progressStage.value = '等待任务…'
    logs.value = ''
    downloadReadyUrl.value = ''
    downloadReadyFileSize.value = 0
    downloadInstallUrl.value = ''
    downloadPackageKind.value = ''
    downloadOtaInstallable.value = false
    downloadInstallMethod.value = ''
    downloadInspection.value = null
    showActionButtons.value = false
    activeDownloadAppId.value = ''
    activeDownloadVersionId.value = ''
    activeDownloadAccountEmail.value = ''
  }

  const checkHttps = () => {
    isHttps.value = window.location.protocol === 'https:'
    currentProtocol.value = window.location.protocol
    return isHttps.value
  }

  // Cleanup on unmount
  onBeforeUnmount(() => {
    if (activeEventSource) {
      activeEventSource.close()
      activeEventSource = null
    }
  })

  return {
    // State
    downloading,
    showProgress,
    progressPercent,
    progressStage,
    logs,
    downloadReadyUrl,
    downloadReadyFileSize,
    downloadInstallUrl,
    downloadPackageKind,
    downloadOtaInstallable,
    downloadInstallMethod,
    downloadInspection,
    showActionButtons,
    activeDownloadAppId,
    activeDownloadVersionId,
    activeDownloadAccountEmail,

    // Purchase state
    checkingPurchaseStatus,
    claimingSelectedApp,
    purchaseStatusText,
    purchaseRequired,
    claimRequired,
    downloadBlocked,
    downloadBlockedReason,
    purchaseActionLabel,

    // Environment
    isHttps,
    currentProtocol,

    // Methods
    startDownloadWithProgress,
    refreshPurchaseStatus,
    preflightPurchaseGate,
    claimSelectedAppInBackground,
    resetDownloadState,
    checkHttps,
    addLog,
    localizeProgressStage,
    formatFileSize,
    openInstallUrl,
    openExternalUrl,
    getSelectedAppPrice
  }
}
