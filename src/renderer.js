const sprite = document.getElementById('pet-sprite');
const bounce = document.getElementById('pet-bounce');

const FRAME_SRC = {
  idle: 'assets/idle.png',
  walk1: 'assets/walk1.png',
  walk2: 'assets/walk2.png',
  walk3: 'assets/walk3.png',
};

let lastSrc = '';
let isJumping = false;

function applyState(state) {
  // 弹跳期间固定显示 idle 素材，不被走路/站立状态更新打断
  const srcKey = isJumping ? 'idle' : (state.walking ? state.frame : 'idle');
  const src = FRAME_SRC[srcKey];
  if (src !== lastSrc) {
    sprite.src = src;
    lastSrc = src;
  }
  sprite.style.transform = state.facingLeft ? 'scaleX(-1)' : 'scaleX(1)';
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

// 是否悬停/点击穿透完全由 Rust 后端根据鼠标是否落在宠物贴图范围内自动处理，
// 前端只需要负责渲染当前状态，以及在真正收到点击时(说明鼠标确实在宠物身上)触发弹跳。
window.__TAURI__.event.listen('pet-state', (event) => {
  applyState(event.payload);
});

sprite.addEventListener('click', triggerJump);
