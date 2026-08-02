# DesktopPet (Tauri)

一个本地运行的透明桌面像素小人宠物。会缓慢跟着鼠标走、点它会跳一下、被鼠标甩开会放弃追逐跑去屏幕底部待机溜达，托盘图标可以隐藏/显示/退出。

## 直接使用（不需要装任何东西）

可执行文件在：

```
src-tauri/target/release/DesktopPet.exe
```

或者双击项目根目录下的 [start.bat](start.bat)，效果一样。

**这是单个文件，不需要安装，双击就能跑；发给别人也只需要这一个 `DesktopPet.exe` 文件。**

使用前注意两点：

1. **需要 WebView2 运行时**：这是 Windows 自带的浏览器组件，Windows 10（较新版本）和 Windows 11 都默认自带，基本不用额外安装。
2. **可能弹出 SmartScreen 警告**：因为这个 exe 没有做代码签名，Windows 第一次运行未知来源的 exe 时可能会提示"Windows 已保护你的电脑"。这是正常现象，点"更多信息" → "仍要运行"即可。

## 功能

- 窗口透明、无边框、始终置顶，只显示宠物本体
- 缓慢跟随鼠标移动（有加速度，不是瞬移），靠近后停在鼠标侧边（不挡住鼠标指向的内容），并转身看向鼠标
- 移动时播放走路动画（人物朝向固定，不会镜像翻转）
- 点击宠物本体会往上弹一下
- 鼠标悬停在宠物身上时会停下，不再追逐
- 鼠标快速甩远（或距离拉得够远）时，宠物会放弃追逐，走回屏幕底部待机；待机一阵子后会慢悠悠地左右溜达，如此循环；鼠标靠近时会重新开始追逐
- 长按宠物本体可以把它拖到屏幕任意位置（含多显示器），带一点"重量感"的跟手滞后，不会跟点击弹跳冲突；松手后自动走回底部，进入待机/溜达循环
- 右键点击宠物本体：弹出一个小窗口显示实时 CPU / 内存占用和电量（笔记本才有电量），再次右键收起
- 系统托盘图标：右键菜单"隐藏/显示"、"退出"；左键单击图标快速隐藏/显示
- 窗口对鼠标点击穿透（除了宠物本体所在的一小块矩形区域），不影响操作桌面其他内容

## 已知问题：拖到另一块屏幕过不去

如果拖拽时鼠标在两块屏幕的交界处卡住、弹不过去，这不是程序的 bug——是 Windows 的**显示器排列设置**跟你实际的物理摆放对不上（有缝隙或者错位）。去"设置 → 系统 → 显示"，把示意图里的两个屏幕图标拖到跟实际摆放贴合（没有缝隙），应用后鼠标就能正常跨屏，宠物也会跟着过去。

## 已知问题：鼠标停在屏幕最顶/最底边缘时宠物会卡住不停走路

追逐鼠标时如果鼠标停在屏幕顶部或底部那一小段"宠物物理上够不到"的区域（因为窗口本身留了一点边距），宠物会一直卡在边界原地"用力"，走路动画不会切回待机。这个问题已经在 `main` / `baseline-simple` / `feature/customizable-assets` 修好了，这个分支还没同步。

## 已知问题：多开时容易叠在一起

这个分支还没有多开防重叠、按贴图像素精确判定点击这些改动（见下面"分支说明"），多开体验会不如 `main`。

## 分支说明

这个仓库有几个并行分支，功能范围不完全一样：

- **main**：主线分支，功能最全，持续在这个基础上继续开发。
- **baseline-simple**：跟 main 保持功能同步的精简版本，专门用来分享给不需要额外功能（自定义皮肤、系统状态弹窗等）的朋友；目前和 main 完全一致。
- **feature/customizable-assets**：在 main 的基础上，加了"不用重新编译，直接换 exe 旁边 assets 文件夹里的图就能换皮肤"的功能。
- **feature/system-stats**：在 main 之前的某个版本基础上，加了这个 README 顶部提到的右键系统状态小窗口功能；还没同步 main 后来加上的多开防重叠、按像素精确点击判定、边界卡死修复这些改动，功能上暂时落后于 main。

## 开发环境搭建

需要：

- [Node.js](https://nodejs.org/)（用于跑 Tauri CLI）
- [Rust 工具链](https://rustup.rs/)
- Windows 上编译还需要 **Visual Studio C++ 生成工具**（MSVC 链接器），可以通过 [Visual Studio Installer](https://visualstudio.microsoft.com/visual-cpp-build-tools/) 安装 "使用 C++ 的桌面开发" 工作负载

```bash
npm install
```

## 本地运行调试

```bash
npm run dev
```

## 重新打包成 exe

```bash
npm run build
```

编译产物在 `src-tauri/target/release/DesktopPet.exe`。

## 项目结构

```
Pet_Tauri/
├── src/                     前端(窗口内容)
│   ├── index.html
│   ├── renderer.js
│   ├── stats.html            系统状态小窗口的页面
│   ├── renderer_stats.js     系统状态小窗口的渲染逻辑
│   └── assets/              五张宠物素材(idle/walk1/walk2/walk3/drag.png)
├── src-tauri/                Rust 后端
│   ├── src/main.rs           入口/托盘/窗口装配，不含具体逻辑
│   ├── src/physics.rs        核心状态机：追逐/待机/溜达/拖拽/跳跃
│   ├── src/input.rs          底层 Win32 输入/显示器信息读取
│   ├── src/stats.rs          系统状态小窗口功能(CPU/RAM/电量)，独立挂在核心状态机上
│   ├── src/util.rs           零散小工具函数
│   ├── tauri.conf.json       窗口/托盘/图标配置
│   ├── icons/                应用图标(由 idle.png 生成)
│   └── target/release/       编译产物(DesktopPet.exe 在这里)
├── package.json
└── start.bat                 双击启动编译好的 exe
```

## 想换个角色贴图/调参数

- 直接替换 `src/assets/` 下的 5 张图（文件名必须是 `idle.png` / `walk1.png` / `walk2.png` / `walk3.png` / `drag.png`，其中 `drag.png` 是长按拖拽时显示的专属贴图），如果新素材的像素尺寸跟原来的 120x165 差异较大，需要同步调整 `src-tauri/src/physics.rs` 顶部的 `SPRITE_W` / `SPRITE_H`，以及 `src/index.html` 里 `#pet img` 的 `max-width` / `max-height`。
- 跟随速度、停靠距离、甩开判定、待机/溜达时间等所有行为参数都集中在 `src-tauri/src/physics.rs` 文件最上面的常量区，改完用 `npm run build` 重新编译即可。
