# Flutter 移植到自制 x86_64 Rust 裸机内核 — 技术调研报告

> 日期：2026-08-24  
> 项目：flutter-os  
> 状态：调研阶段，未开始移植实现

---

## 1. flutter-pi 工作原理

### 1.1 定位

[flutter-pi](https://github.com/ardera/flutter-pi) 是一个轻量级 Flutter Engine Embedder，面向 Raspberry Pi / Linux 直出显示，**不使用 X11 桌面环境**。它在没有 Wayland/X11 时直接启动 Flutter 应用，但依赖硬件 3D 加速和 KMS/DRI [$TRAE_REF](https://github.com/ardera/flutter-pi)。

支持条件：
- 硬件 3D 加速
- Kernel Mode Setting (KMS)
- Direct Rendering Infrastructure (DRI)
- CPU 架构：ARMv7、ARMv8、x86、x86_64

### 1.2 架构

```
Flutter app bundle
  ├── Dart/Flutter framework
  ├── Flutter assets
  ├── app.so (AOT snapshot, release/profile 模式)
  └── icudtl.dat

flutter-pi embedder
  ├── 加载 libflutter_engine.so
  ├── 创建 DRM/KMS 输出 (modesetting.c)
  ├── 创建 GBM/EGL/GLES 渲染环境 (gl_renderer.c)
  ├── 处理触摸/鼠标/键盘输入 (libinput)
  ├── 创建 Flutter backing stores
  ├── 接收 Flutter compositor layer 合成结果
  └── 通过 DRM/KMS page flip 显示
```

源码结构 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/flutter-pi.c):
- `flutter-pi.c` — 主入口
- `gl_renderer.c` — GBM + EGL + GLES 渲染 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/gl_renderer.c)
- `modesetting.c` — DRM/KMS 模式设置 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/modesetting.c)
- `compositor_ng.c` — 合成器 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/compositor_ng.c)

### 1.3 核心渲染流程

`gl_renderer.c` 使用 `EGL_PLATFORM_GBM_KHR` 从 GBM device 创建 EGL display:

```c
egl_get_platform_display("eglGetPlatformDisplay");
egl_display = egl_get_platform_display(EGL_PLATFORM_GBM_KHR, gbm_device, NULL);
root_context = eglCreateContext(egl_display, config, EGL_NO_CONTEXT, context_attribs);
```

检查必要扩展: `EGL_KHR_surfaceless_context`, `EGL_KHR_platform_gbm`

### 1.4 构建依赖

```
cmake
libgl1-mesa-dev, libgles2-mesa-dev, libegl1-mesa-dev
libdrm-dev, libgbm-dev
libsystemd-dev, libinput-dev, libudev-dev, libxkbcommon-dev
ttf-mscorefonts-installer, fontconfig
```

运行时需要: `libflutter_engine.so.{debug,profile,release}`, `icudtl.dat`

---

## 2. flutter-elinux 工作原理

### 2.1 定位

[flutter-elinux](https://github.com/sony/flutter-elinux) 是 Sony 提供的非官方 Flutter SDK 扩展，用于嵌入式 Linux 设备。使用 [flutter-embedded-linux](https://github.com/sony/flutter-embedded-linux) embedder [$TRAE_REF](https://github.com/sony/flutter-elinux)。

特点：比桌面 Flutter for Linux 更轻量，不使用 X11 和 GTK，支持 arm64/x64 [$TRAE_REF](https://github.com/sony/flutter-elinux)。

### 2.2 支持后端

| 后端 | 依赖 |
|------|------|
| Wayland | libwayland, wayland-protocols, Weston/Sway compositor |
| DRM-GBM | libdrm, libgbm, libinput, libudev, libsystemd 或 libuv |
| DRM-EGLStream | NVIDIA 专用 |
| X11 | libx11-dev (仅调试) |

### 2.3 最小依赖

```bash
sudo apt install clang cmake build-essential pkg-config \
    libegl1-mesa-dev libxkbcommon-dev libgles2-mesa-dev
```

### 2.4 与 flutter-pi 的差异

| 维度 | flutter-pi | flutter-elinux |
|------|-----------|----------------|
| 目标 | RPi / 轻量直出 | 通用嵌入式 Linux SDK |
| 后端 | DRM/KMS + GBM + EGL + GLES | Wayland、DRM-GBM、DRM-EGLStream、X11 |
| 输入 | libinput, udev, xkbcommon | 类似 |
| SDK | 自建 embedder | Sony 提供完整 SDK |
| 运行时 | libflutter_engine.so + icudtl.dat | 同左 |

---

## 3. Flutter Engine 平台依赖清单

### 3.1 ELF 动态链接器

`libflutter_engine.so` 是动态链接 ELF shared object，运行时需要:
- `/lib/ld-linux-x86-64.so.2` (动态链接器)
- glibc 或 musl libc
- ELF RPATH/RUNPATH 库搜索路径 [$TRAE_REF](https://manpages.ubuntu.com/manpages/bionic/man8/ld-linux.8.html)

### 3.2 libc 函数依赖

Flutter Engine 内部 C++ 代码需要:
```
malloc, calloc, free, realloc
strlen, strcmp, memcpy, memset
open, read, write, close, stat, fstat
mmap, munmap, mprotect
pthread_create, pthread_join, pthread_mutex_*, pthread_cond_*
dlopen, dlsym, dlerror
epoll_create1, epoll_ctl, epoll_wait
eventfd, timerfd
ioctl
```

### 3.3 GPU/图形依赖

需要完整的 Linux GPU stack:
```
libEGL.so    — EGL 上下文管理
libGLESv2.so — OpenGL ES 2/3 渲染
libdrm.so    — DRM/KMS 模式设置
libgbm.so    — GBM buffer 管理
```

DRM ioctl 调用 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/modesetting.c):
```
DRM_IOCTL_MODE_GETRESOURCES
DRM_IOCTL_MODE_GETCONNECTOR
DRM_IOCTL_MODE_GETCRTC
DRM_IOCTL_MODE_SETCRTC
DRM_IOCTL_MODE_ADDFB2
DRM_IOCTL_MODE_PAGE_FLIP
DRM_IOCTL_MODE_CREATE_DUMB
DRM_IOCTL_MODE_MAP_DUMB
```

### 3.4 输入子系统

```
/dev/input/event*  — libinput + udev
libxkbcommon       — 键盘映射
epoll              — 事件循环
```

### 3.5 资源文件

```
icudtl.dat         — ICU 国际化数据
fonts/             — 字体文件 (Arial 等)
flutter_assets/    — Flutter 应用资源
app.so             — AOT 编译的 Dart 代码
```

---

## 4. 移植到自制无 libc 裸机内核的依赖清单

### 4.1 必须实现的内核子系统

| 子系统 | 当前状态 | 需要实现 |
|--------|---------|---------|
| Ring3 用户态 | `[PENDING]` | TSS user segments, `iretq` 到 Ring3, 用户栈 |
| 进程调度 | `[PENDING]` | 任务结构体, 上下文切换, 调度器 |
| ELF 加载器 | 未开始 | 解析 ELF, 映射段, 设置入口 |
| 动态链接器 | 未开始 | `ld.so` 等价物, 符号解析, 重定位 |
| libc 兼容层 | 未开始 | malloc/free, string, stdio |
| pthread | 未开始 | 线程创建/销毁, mutex, condvar |
| epoll/eventfd | 未开始 | 事件循环 |
| DRM/KMS | 未开始 | DRM ioctl 兼容, page flip |
| GBM | 未开始 | buffer 管理 |
| EGL/GLES | 未开始 | OpenGL ES 软件渲染或硬件加速 |
| FAT32/文件系统 | `[PENDING]` | 完整 VFS, 路径解析 |
| 输入子系统 | 部分 (PS/2) | libinput 等价物, /dev/input |
| mmap | `[OK]` (骨架) | 真正虚拟内存映射 |
| 时间子系统 | `[OK]` | 已实现 PIT tick + clock_gettime |

### 4.2 最小 POSIX syscall 表面

运行 Flutter Engine 需要的 syscall 超集:

```
// 已实现 (骨架)
open, read, write, mmap, nanosleep, clock_gettime

// 必须新增
close, fstat, lstat, access, dup, dup2, fcntl
munmap, mprotect, brk
clone, exit, waitpid
epoll_create1, epoll_ctl, epoll_wait, eventfd, timerfd
ioctl (DRM)
sched_yield
gettid, getpid
pipe, pipe2
```

参考: `mmap(2)` man page [$TRAE_REF](https://www.man7.org/linux/man-pages/man2/mmap.2.html), `dlopen` [$TRAE_REF](https://www.mankier.com/3/dlopen)

---

## 5. 两条实现路线对比

### 路线 A：裸机直接跑 Flutter Engine

| 维度 | 评估 |
|------|------|
| **难度** | ★★★★★ (极高) |
| **核心障碍** | 需要从零实现 ELF 动态链接器 + libc + pthread + epoll + DRM/KMS + EGL/GLES |
| **优点** | 无中间层，性能最优，架构最干净 |
| **缺点** | 工作量巨大 (估计 6-12 个月), 需要实现几乎完整的 Linux 兼容层 |
| **障碍清单** | 1. 没有 ELF 动态链接器 — `libflutter_engine.so` 无法加载 |
|              | 2. 没有 libc — 所有 C 标准库函数缺失 |
|              | 3. 没有 pthread — Flutter Engine 的多线程无法工作 |
|              | 4. 没有 Linux GPU stack — DRM/GBM/EGL/GLES 全部缺失 |
|              | 5. 没有 `/dev/dri/card0` — 无法渲染 |
|              | 6. 没有 epoll/eventfd — 事件循环无法工作 |
|              | 7. 字体/ICU 加载路径不存在 |

### 路线 B：内核作为 Hypervisor，跑最小 Linux 子环境

| 维度 | 评估 |
|------|------|
| **难度** | ★★★☆☆ (中等) |
| **核心思路** | Rust 内核实现极简类 Linux 用户态 ABI, 运行最小嵌入式 Linux (如 Buildroot/Alpine), 在子环境运行 flutter-pi |
| **优点** | 复用 Linux 生态所有库, flutter-pi 可直接运行, 开发周期短 |
| **缺点** | 引入 Linux 子环境复杂度, 性能有虚拟化开销 |
| **实现方式** | 方案 B1: 真正虚拟化 (KVM-like) — 内核实现 VT-x, 运行 Linux guest |
|              | 方案 B2: 容器化 — 内核实现足够 Linux ABI, 直接运行 Linux 静态二进制 |
|              | 方案 B3: unikernel — 用 rumprun/IncludeOS 等库 OS 模式 |

### 路线 C（补充推荐）：重写 Flutter Embedder

| 维度 | 评估 |
|------|------|
| **难度** | ★★★★☆ (高) |
| **核心思路** | 不使用 `libflutter_engine.so`, 参考 flutter-pi 架构, 用 Rust 重写 embedder + 渲染后端 |
| **优点** | 最可控, 可逐步推进, 不需要完整 Linux 兼容层 |
| **缺点** | 需要深入 Flutter Engine 内部, 渲染需要自己实现 (软件光栅化或简单 GLES) |
| **可行性** | 中期可行 — 先实现软件渲染, 后续接 GPU |

---

## 6. 分步开发路线图

### 阶段 0: 当前已完成 ✅

- [x] GDT + IDT + 8259 PIC
- [x] 内核堆分配 (1 MiB)
- [x] VGA 文本输出 + COM1 串口
- [x] PS/2 键盘输入
- [x] framebuffer 帧缓冲图形 (画点/矩形/渐变)
- [x] Syscall 框架骨架 (open/read/write/mmap/nanosleep/clock_gettime)
- [x] 时间子系统 (PIT tick + clock_gettime)
- [x] CI 自动化构建 + QEMU 启动测试

### 阶段 1: Ring3 用户态 (AI 可生成骨架 + 需手动调试)

- [ ] TSS user segments (DS/CS for Ring3)
- [ ] `iretq` 从内核进入 Ring3
- [ ] 用户态栈分配
- [ ] `int 0x80` 或 `syscall` 指令进入内核
- [ ] 系统调用寄存器提取 + 返回值设置
- **AI 可生成**: IDT 门设置代码, 寄存器保存/恢复
- **必须手动**: CPL 切换调试, 页面权限设置

### 阶段 2: 进程/线程管理 (AI 可生成 + 需手动调试)

- [ ] Process 结构体 (页表, FD 表, 信号掩码)
- [ ] 上下文切换 (保存/恢复 callee-saved 寄存器)
- [ ] 简单轮转调度器
- [ ] pthread 兼容 (线程 = 共享地址空间的进程)
- **AI 可生成**: 任务结构体, 调度器骨架
- **必须手动**: 上下文切换汇编, 栈切换调试

### 阶段 3: 虚拟内存 + mmap (AI 可生成 + 需手动调试)

- [ ] 4 级页表管理 (PML4 → PDPT → PD → PT)
- [ ] 物理帧分配器 (buddy allocator)
- [ ] `mmap` 真实实现 (分配虚拟区域 + 映射物理帧)
- [ ] `munmap`, `mprotect`
- **AI 可生成**: 页表遍历代码, 帧分配器
- **必须手动**: 页错误处理, TLB shootdown

### 阶段 4: 文件系统 (AI 可生成 + 需手动调试)

- [ ] ATA/ATAPI 磁盘驱动 (PIO 模式)
- [ ] FAT32 只读文件系统
- [ ] VFS 抽象层
- [ ] `open`, `read`, `close`, `stat` 完整实现
- **AI 可生成**: FAT32 解析, ATA 寄存器操作
- **必须手动**: 磁盘 I/O 时序, 错误恢复

### 阶段 5: 事件循环 + epoll (AI 可生成)

- [ ] `eventfd` 实现
- [ ] `epoll_create1`, `epoll_ctl`, `epoll_wait`
- [ ] `timerfd` 实现
- [ ] `pipe`/`pipe2`
- **AI 可生成**: 全部 (纯软件逻辑)
- **必须手动**: 与中断子系统的集成

### 阶段 6: ELF 加载器 (AI 可生成 + 需手动调试)

- [ ] ELF64 解析 (header, program headers, sections)
- [ ] 段映射 (LOAD segments → mmap)
- [ ] 动态链接器 (`.dynamic`, `.dynsym`, `.rela`)
- [ ] `dlopen`, `dlsym`, `dlerror`
- **AI 可生成**: ELF 解析, 符号表遍历
- **必须手动**: 重定位类型处理, PLT/GOT 设置

### 阶段 7: libc 兼容层 (AI 可生成 + 需手动调试)

- [ ] malloc/free (已有 heap, 需 ptmalloc/jemalloc)
- [ ] string.h (memcpy 等已有)
- [ ] stdio (printf, fprintf → write syscall)
- [ ] errno 支持
- **AI 可生成**: 大部分 (纯软件)
- **必须手动**: 线程安全的 malloc

### 阶段 8: 图形渲染 (需手动开发为主)

- [ ] 路线 A: 实现 DRM ioctl 兼容 + GBM + 软件 EGL/GLES
- [ ] 路线 B: 准备最小 Linux 镜像 (Buildroot), 集成 flutter-pi
- [ ] 路线 C: 用 Rust 重写 embedder + 软件光栅化
- **AI 可生成**: 软件光栅化器 (线段/三角形)
- **必须手动**: GPU 驱动 (如果走硬件加速路线)

### 阶段 9: Flutter Engine 运行 (需手动调试)

- [ ] 加载 `libflutter_engine.so`
- [ ] 加载 `icudtl.dat`
- [ ] 创建 Flutter Engine embedder API
- [ ] 渲染 backing store
- [ ] 输入事件传递
- **AI 可生成**: embedder API 调用代码
- **必须手动**: Skia/Impeller 渲染后端集成

---

## 7. 推荐路线

**短期 (3-6 个月)**: 路线 B2 (容器化) — 实现足够 Linux ABI, 运行静态链接的 flutter-pi

**中期 (6-12 个月)**: 路线 C (重写 embedder) — 参考 flutter-pi 架构, 用 Rust 重写

**长期 (12+ 个月)**: 路线 A (裸机直跑) — 完整 Linux 兼容层

---

## 8. 当前内核能力 vs Flutter 需求差距

| Flutter 需求 | 当前内核状态 | 差距 |
|-------------|-------------|------|
| ELF 动态链接 | ❌ 无 | 需实现完整 ld.so |
| libc | ❌ 无 | 需实现 malloc, string, stdio |
| pthread | ❌ 无 | 需实现线程 + 同步原语 |
| epoll/eventfd | ❌ 无 | 需实现事件循环 |
| DRM/KMS | ❌ 无 | 需实现 DRM ioctl 兼容 |
| GBM | ❌ 无 | 需实现 buffer 管理 |
| EGL/GLES | ❌ 无 | 需实现或移植 |
| FAT32 文件 | ❌ 无 | 需实现 ATA + FAT32 |
| Ring3 用户态 | `[PENDING]` | 需实现 TSS + iretq |
| mmap | `[OK]` 骨架 | 需实现虚拟内存映射 |
| clock_gettime | `[OK]` | ✅ 已满足 |
| write | `[OK]` | ✅ 已满足 (stdout) |
| PS/2 键盘 | `[OK]` | 需包装为 /dev/input |

---

## 9. 技术约束声明

1. **不使用虚构 crate**: 所有依赖均为真实存在的 crate (bootloader, x86_64, spin, pic8259, uart_16550, lazy_static, linked_list_allocator)
2. **区分骨架与实现**: syscall 的 `open`/`mmap` 为骨架 (返回 ENOSYS), `write`/`clock_gettime`/`nanosleep` 为可用实现
3. **Ring3 未实现**: 当前所有代码运行在 Ring0, 无法运行用户态程序
4. **无 GPU**: 当前渲染为 framebuffer 软件 blit, 无 OpenGL ES 支持
5. **CI 限制**: GitHub runner 无 KVM, QEMU 使用 TCG 软件模拟

---

## 10. 关键引用

- flutter-pi: https://github.com/ardera/flutter-pi
- flutter-pi 源码: https://github.com/ardera/flutter-pi/tree/master/src
- flutter-elinux: https://github.com/sony/flutter-elinux
- flutter-embedded-linux: https://github.com/sony/flutter-embedded-linux
- flutter-elinux Wiki: https://github.com/sony/flutter-elinux/wiki/How-to-use-Flutter-for-Embedded-Linux
- Linux 动态链接器: https://manpages.ubuntu.com/manpages/bionic/man8/ld-linux.8.html
- mmap(2): https://www.man7.org/linux/man-pages/man2/mmap.2.html
- dlopen(3): https://www.mankier.com/3/dlopen
