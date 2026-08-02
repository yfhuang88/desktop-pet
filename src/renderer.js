const sprite = document.getElementById('pet-sprite');
const bounce = document.getElementById('pet-bounce');

// 先用内置默认图占位，等下面的 get_skin_config 调用返回后，
// 如果 exe 同目录 assets/ 文件夹里有自定义图片，会直接覆盖这几个字段的值。
// 之后的每一帧 applyState 都会重新从这个表里取值，所以覆盖会在下一帧自动生效，
// 不需要额外的"刷新"逻辑。
const FRAME_SRC = {
  idle: 'assets/idle.png',
  walk1: 'assets/walk1.png',
  walk2: 'assets/walk2.png',
  walk3: 'assets/walk3.png',
  drag: 'assets/drag.png',
};

window.__TAURI__.tauri.invoke('get_skin_config').then((config) => {
  FRAME_SRC.idle = config.idleSrc;
  FRAME_SRC.walk1 = config.walk1Src;
  FRAME_SRC.walk2 = config.walk2Src;
  FRAME_SRC.walk3 = config.walk3Src;
  FRAME_SRC.drag = config.dragSrc;
});

let lastSrc = '';
let isJumping = false;

function applyState(state) {
  // 弹跳期间固定显示 idle 素材，不被走路/站立状态更新打断；
  // 拖拽期间显示专属的 drag 素材，优先级低于弹跳、高于走路/待机。
  const srcKey = isJumping
    ? 'idle'
    : state.dragging
      ? 'drag'
      : (state.walking ? state.frame : 'idle');
  const src = FRAME_SRC[srcKey];
  if (src !== lastSrc) {
    sprite.src = src;
    lastSrc = src;
  }
  // 不再根据移动方向左右镜像翻转贴图，人物始终保持原朝向
}

function triggerJump() {
  if (isJumping) return;
  isJumping = true;
  if (lastSrc !== FRAME_SRC.idle) {
    sprite.src = FRAME_SRC.idle;
    lastSrc = FRAME_SRC.idle;
  }
  bounce.classList.add('jumping');
}

bounce.addEventListener('animationend', () => {
  bounce.classList.remove('jumping');
  isJumping = false;
});

// 是否悬停/点击穿透，以及"这是一次点击还是一次长按拖拽"，都由 Rust 后端
// 统一判定(它同时掌握鼠标左键状态和光标位置)。前端只负责纯渲染：
// 收到 pet-state 就更新贴图/朝向，收到 pet-jump 就播放弹跳动画。
// 拖拽本身也完全由后端直接搬动窗口位置实现，前端不需要处理任何拖拽逻辑。
window.__TAURI__.event.listen('pet-state', (event) => {
  applyState(event.payload);
});

window.__TAURI__.event.listen('pet-jump', () => {
  triggerJump();
});
