'use strict';

// ── Tauri v2 API ─────────────────────────────────────────────────────────────
const { invoke } = window.__TAURI__.core;
const { listen }  = window.__TAURI__.event;

// ── State ─────────────────────────────────────────────────────────────────────
let mode      = 'manual';
let isRunning = false;

// ── Init ──────────────────────────────────────────────────────────────────────
window.addEventListener('DOMContentLoaded', async () => {
    // Version from Cargo.toml via command
    try {
        const ver = await invoke('get_version');
        document.getElementById('appVersion').textContent = 'v' + ver;
    } catch (_) {}

    // Local IP
    try {
        const ip = await invoke('get_local_ip_addr');
        document.getElementById('localIp').textContent = ip;
    } catch (_) {}

    // Update check
    try {
        const info = await invoke('check_updates');
        if (info) {
            const badge = document.getElementById('updateBadge');
            badge.textContent = `Обновление: ${info.tag}`;
            badge.hidden = false;
            badge.onclick = (e) => { e.preventDefault(); invoke('open_url', { url: info.link }); };
        }
    } catch (_) {}

    // Sync port range visibility with initial checkbox state
    togglePortRange();

    // Listen for log entries from the backend
    await listen('proxy-log', (evt) => {
        addLog(evt.payload.level, evt.payload.message);
    });

    // session-started is now handled directly in startSession() for instant
    // feedback; keep this listener only as a safety-net (e.g. external trigger).
    await listen('session-started', () => { setRunning(true); });

    await listen('session-ended', () => {
        setRunning(false);
        setClientStatus('primary',   false);
        setClientStatus('secondary', false);
    });

    await listen('client-status', (evt) => {
        setClientStatus(evt.payload.which, evt.payload.online);
    });
});

// ── Mode selection ────────────────────────────────────────────────────────────
function setMode(m) {
    mode = m;
    document.getElementById('tabManual').classList.toggle('active', m === 'manual');
    document.getElementById('tabAuto').classList.toggle('active', m === 'auto');
    document.getElementById('serverSection').hidden = m !== 'manual';
    document.getElementById('autoInfo').hidden      = m !== 'auto';
}

function togglePortRange() {
    const checked = document.getElementById('windivertCheck').checked;
    document.getElementById('portRangeRow').style.display = checked ? 'flex' : 'none';
}

// ── Session control ───────────────────────────────────────────────────────────
async function startSession() {
    const addr = document.getElementById('serverAddr').value.trim();

    if (mode === 'manual' && !addr) {
        addLog('error', 'Введите адрес сервера');
        return;
    }

    try {
        if (mode === 'manual') {
            await invoke('start_manual_session', { serverAddr: addr });
        } else {
            const useWindivert = document.getElementById('windivertCheck').checked;
            let portMin = 25560, portMax = 25570;
            if (useWindivert) {
                portMin = parseInt(document.getElementById('portMin').value, 10);
                portMax = parseInt(document.getElementById('portMax').value, 10);
                if (isNaN(portMin) || isNaN(portMax) || portMin < 1 || portMax > 65535 || portMin >= portMax) {
                    addLog('error', 'Неверный диапазон портов (min < max, 1–65535)');
                    return;
                }
            }
            await invoke('start_auto_session', { useWindivert, portMin, portMax });
        }
        // Flip the button immediately — don't wait for the event.
        setRunning(true);
    } catch (err) {
        addLog('error', String(err));
        // Make sure the button stays in the correct state on error.
        setRunning(false);
    }
}

async function stopSession() {
    try {
        await invoke('stop_session');
    } catch (err) {
        addLog('error', String(err));
    }
}

// ── UI helpers ────────────────────────────────────────────────────────────────
function setRunning(running) {
    isRunning = running;
    document.getElementById('startBtn').hidden = running;
    document.getElementById('stopBtn').hidden  = !running;
    document.getElementById('serverAddr').disabled    = running;
    document.getElementById('tabManual').disabled     = running;
    document.getElementById('tabAuto').disabled       = running;
    document.getElementById('windivertCheck').disabled = running;
    document.getElementById('portMin').disabled       = running;
    document.getElementById('portMax').disabled       = running;
}

function setClientStatus(which, online) {
    const dot    = document.getElementById(which + 'Dot');
    const status = document.getElementById(which + 'Status');
    dot.classList.toggle('online', online);
    status.textContent = online ? 'online' : 'offline';
}

function addLog(level, message) {
    const prefixes = { success: '[+]', info: '[~]', warn: '[!]', error: '[!]' };
    const prefix   = prefixes[level] ?? '[?]';

    const log = document.getElementById('logOutput');
    const el  = document.createElement('div');
    el.className   = `log-entry ${level}`;
    el.textContent = `${prefix} ${message}`;
    log.appendChild(el);
    log.scrollTop = log.scrollHeight;
}

function clearLog() {
    document.getElementById('logOutput').innerHTML = '';
}
