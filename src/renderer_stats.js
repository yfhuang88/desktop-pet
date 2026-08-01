const cpuBar = document.getElementById('cpu-bar');
const cpuValue = document.getElementById('cpu-value');
const cpuRow = document.getElementById('cpu-row');
const ramBar = document.getElementById('ram-bar');
const ramValue = document.getElementById('ram-value');
const ramRow = document.getElementById('ram-row');
const batteryRow = document.getElementById('battery-row');
const batteryBar = document.getElementById('battery-bar');
const batteryValue = document.getElementById('battery-value');
const batteryName = document.getElementById('battery-name');

// 占位配色分级：<60% 绿、60-85% 黄、>85% 红；正式美术定稿后这里的阈值/颜色再统一调整
function levelClass(percent) {
  if (percent > 85) return 'level-high';
  if (percent > 60) return 'level-mid';
  return 'level-low';
}

function applyRow(row, bar, valueEl, percent, label) {
  row.classList.remove('level-low', 'level-mid', 'level-high');
  row.classList.add(levelClass(percent));
  bar.style.width = `${Math.max(0, Math.min(100, percent)).toFixed(0)}%`;
  valueEl.textContent = `${label !== undefined ? label : percent.toFixed(0) + '%'}`;
}

window.__TAURI__.event.listen('stats-update', (event) => {
  const { cpuPercent, ramPercent, batteryPercent, onAcPower } = event.payload;

  applyRow(cpuRow, cpuBar, cpuValue, cpuPercent);
  applyRow(ramRow, ramBar, ramValue, ramPercent);

  if (batteryPercent === null || batteryPercent === undefined) {
    batteryRow.style.display = 'none';
  } else {
    batteryRow.style.display = '';
    batteryName.textContent = onAcPower ? '电量 (充电中)' : '电量';
    applyRow(batteryRow, batteryBar, batteryValue, batteryPercent);
  }
});

document.addEventListener('contextmenu', (e) => e.preventDefault());
