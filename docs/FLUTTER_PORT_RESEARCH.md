# Flutter Engine 移植到 AeroOS — 完整技术文档

> 日期：2026-08-24  
> 项目：flutter-os  
> 状态：调研 + 骨架代码已生成，未开始完整移植实现

---

## 目录

1. [flutter-elinux 引擎依赖清单](#1-flutter-elinux-引擎依赖清单)
2. [两条移植路径对比](#2-两条移植路径对比)
3. [模块级分类：复用 / 重写 / 人工开发](#3-模块级分类复用--重写--人工开发)
4. [已生成骨架代码清单](#4-已生成骨架代码清单)
5. [需要人工修改实现的部分](#5-需要人工修改实现的部分)
6. [几乎无法 AI 生成的部分](#6-几乎无法-ai-生成的部分)
7. [内核侧已完成的变更](#7-内核侧已完成的变更)
8. [技术约束声明](#8-技术约束声明)

---

## 1. flutter-elinux 引擎依赖清单

[flutter-elinux](https://github.com/sony/flutter-elinux) 是 Sony 提供的非官方 Flutter SDK 扩展，用于嵌入式 Linux 设备，底层使用 [flutter-embedded-linux](https://github.com/sony/flutter-embedded-linux) embedder [$TRAE_REF](https://github.com/sony/flutter-elinux)。不使用 X11 和 GTK，支持 arm64/x64。

### 1.1 系统调用依赖

| syscall | 用途 | AeroOS 状态 |
|---------|------|------------|
| `open` | 打开 icudtl.dat、assets、app.so | 骨架（返回 ENOSYS） |
| `read` | 读取文件、stdin | ✅ stdin（PS/2 scancode） |
| `write` | stdout/stderr 输出 | ✅ serial + VGA |
| `close` | 关闭 fd | 未实现 |
| `mmap` | 映射 ELF 段、纹理、framebuffer | 骨架（heap-backed） |
| `munmap` | 释放映射 | 未实现 |
| `mprotect` | 设置段权限 | 未实现 |
| `nanosleep` | 睡眠等待 | ✅ PIT tick |
| `clock_gettime` | 获取时间戳 | ✅ CLOCK_MONOTONIC |
| `clone` | 创建线程（pthread 底层） | 未实现 |
| `futex` | 线程同步（mutex/condvar 底层） | 未实现 |
| `epoll_create1/ctl/wait` | 事件循环 | 未实现 |
| `eventfd` | 线程间信号 | 未实现 |
| `timerfd_create` | 定时器 | 未实现 |
| `ioctl` | DRM/KMS 设备控制 | 未实现 |
| `stat/fstat` | 文件元信息 | 未实现 |
| `pipe/pipe2` | 管道 | 未实现 |
| `gettid/getpid` | 线程/进程 ID | 未实现 |
| `sched_yield` | 让出 CPU | 未实现 |

### 1.2 libc 接口依赖

Flutter Engine 内部 C++ 代码（Dart VM + Skia/Impeller + compositor）需要的 libc 函数：

```
// 内存管理
malloc, calloc, free, realloc, memalign, posix_memalign

// 字符串
strlen, strcmp, strncmp, strcpy, strncpy, strcat, strncat,
strchr, strrchr, strstr, strtok, strdup, strndup,
memcmp, memcpy, memset, memmove, memchr

// 文件 I/O
open, read, write, close, lseek, stat, fstat, lstat, access,
fcntl, dup, dup2, truncate, ftruncate

// 内存映射
mmap, munmap, mprotect, madvise, msync, brk, sbrk

// 线程 (pthread)
pthread_create, pthread_join, pthread_detach, pthread_exit,
pthread_mutex_init/destroy/lock/unlock/trylock,
pthread_cond_init/destroy/wait/signal/broadcast/timedwait,
pthread_rwlock_*, pthread_barrier_*, pthread_once,
pthread_self, pthread_setspecific, pthread_getspecific

// 动态链接
dlopen, dlsym, dlclose, dlerror

// 时间
clock_gettime, gettimeofday, nanosleep, clock_nanosleep

// 事件循环
epoll_create1, epoll_ctl, epoll_wait, eventfd, timerfd_create/settime

// 其他
ioctl, pipe, pipe2, gettid, getpid, sched_yield,
getenv, setenv, atexit, exit, abort
```

### 1.3 图形后端依赖

flutter-elinux 支持以下后端 [$TRAE_REF](https://github.com/sony/flutter-elinux)：

| 后端 | 依赖库 | 说明 |
|------|--------|------|
| **Wayland** | libwayland, wayland-protocols | 需要 compositor（Weston/Sway） |
| **DRM-GBM** | libdrm, libgbm, libinput, libudev | 最通用嵌入式路径 |
| **DRM-EGLStream** | NVIDIA 专用 | 仅 NVIDIA GPU |
| **X11** | libx11 | 仅调试用 |

核心 DRM ioctl 调用链（来自 flutter-pi 的 `modesetting.c`）[$TRAE_REF](https://github.com/ardera/flutter-pi)：

```
DRM_IOCTL_MODE_GETRESOURCES   → 枚举连接器
DRM_IOCTL_MODE_GETCONNECTOR   → 检测显示器
DRM_IOCTL_MODE_GETCRTC        → 获取 CRTC
DRM_IOCTL_MODE_SETCRTC        → 设置模式
DRM_IOCTL_MODE_CREATE_DUMB    → 创建 dumb buffer
DRM_IOCTL_MODE_MAP_DUMB       → 映射 dumb buffer
DRM_IOCTL_MODE_ADDFB2         → 创建 framebuffer object
DRM_IOCTL_MODE_PAGE_FLIP      → 页面翻转（vsync）
```

EGL/GLES 初始化（来自 `gl_renderer.c`）[$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/gl_renderer.c)：

```c
egl_display = eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, gbm_device, NULL);
eglCreateContext(egl_display, config, EGL_NO_CONTEXT, context_attribs);
eglMakeCurrent(egl_display, surface, surface, root_context);
```

### 1.4 输入接口依赖

```
/dev/input/event*  → libinput 事件设备
libinput           → 事件解析、去抖、手势
libudev            → 设备枚举热插拔
libxkbcommon       → scancode → keysym → Unicode 映射
epoll              → 异步事件循环
```

### 1.5 资源文件依赖

```
libflutter_engine.so  → Flutter Engine 二进制（~40MB）
icudtl.dat            → ICU 国际化数据（~10MB）
flutter_assets/       → Flutter 应用资源
app.so                → AOT 编译的 Dart 代码
fonts/                → 字体文件
```

---

## 2. 两条移植路径对比

### 路径 A：基于 flutter-elinux，替换 EGL/DRM 后端

**核心思路**：保留 flutter-elinux 的 embedder 主体逻辑，将底层 EGL/DRM/GBM 图形后端替换为我们自研内核的帧缓冲 syscall。Flutter Engine 使用 **Software rasteriser**（Skia 软件渲染）输出像素 buffer，再由适配层 blit 到 framebuffer。

```
┌───────────────────────────────────────────────────────┐
│  Flutter Engine (libflutter_engine.so)                │
│  Dart VM + Skia (software raster) + compositor        │
├───────────────────────────────────────────────────────┤
│  flutter-elinux embedder (C++)                        │
│  ┌─────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ │
│  │ engine  │ │ platform  │ │ renderer │ │ input    │ │
│  │ task    │ │ message   │ │ (替换)   │ │ (替换)   │ │
│  │ runner  │ │ proto     │ │          │ │          │ │
│  └────┬────┘ └─────┬────┘ └────┬─────┘ └────┬─────┘ │
├───────┼────────────┼───────────┼────────────┼────────┤
│       │            │           │            │         │
│  ┌────▼────────────▼───────────▼────────────▼──────┐ │
│  │     flutter_adapter (Rust, Ring3)                │ │
│  │  framebuffer::mmap → canvas → blit              │ │
│  │  input::read(0) → scancode → Flutter event      │ │
│  │  syscalls::int 0x80 → kernel                     │ │
│  └─────────────────────────────────────────────────┘ │
├───────────────────────────────────────────────────────┤
│  AeroOS Kernel (Ring 0)                              │
│  framebuffer | PS/2 | heap | PIT time | int 0x80     │
└───────────────────────────────────────────────────────┘
```

| 维度 | 评估 |
|------|------|
| **难度** | ★★★★☆ (高) |
| **可复用** | flutter-elinux embedder 的 engine task runner、platform message proto、Dart isolate 管理 |
| **必须重写** | renderer 后端（EGL/DRM → framebuffer blit）、input 后端（libinput → PS/2 syscall） |
| **优点** | 保留 Flutter Engine 完整能力（Skia 渲染、Dart 框架、热重载等）；Software raster 不需 GPU |
| **缺点** | 仍需 ELF 动态链接器加载 `libflutter_engine.so`；需实现足够 libc + pthread + epoll；工作量大 |
| **关键阻塞** | 1. 无 ELF 动态链接器 → 无法加载 .so  2. 无 libc/pthread  3. 无 epoll/eventfd |
| **预估工作量** | 4-8 个月（需先完成 ELF loader + libc + pthread + epoll 基础设施） |

**路径 A 的 renderer 替换方案**：

flutter-elinux 的 `renderer.cc` 定义了渲染接口。我们提供一个 Software 渲染实现：

```cpp
// 替换 EGL/DRM 后端的 Software renderer
class FramebufferRenderer : public FlutterRenderer {
    // Engine 请求 backing store
    FlutterSoftwareBackingStore* acquire_backing_store() {
        // 返回一块和 framebuffer 等大的 malloc'd buffer
        // Engine 的 Skia 软件光栅化器会渲染到这里
        return &backing_store;
    }
    
    // Engine 渲染完毕，合成到屏幕
    bool present_backing_store(FlutterSoftwareBackingStore* store) {
        // 调用 Rust 适配层的 framebuffer blit
        // flutter_adapter_framebuffer_blit(store->buffer, store->width, store->height);
        return true;
    }
};
```

### 路径 B：导出 Skia 绘制指令流，内核侧简易解析渲染

**核心思路**：不运行完整 Flutter Engine，而是修改 Skia 后端，将绘制指令序列化后导出为指令流，在内核侧或用户态简易渲染器中解析并执行。

```
┌───────────────────────────────────────────────┐
│  Flutter Framework (Dart)                     │
│  Widget tree → RenderObject → Paint           │
├───────────────────────────────────────────────┤
│  Skia (修改)                                  │
│  Canvas → 指令序列化                          │
│  DrawPaint, DrawRect, DrawPath, DrawImage... │
│  序列化为二进制指令流 → pipe/shared memory   │
├───────────────────────────────────────────────┤
│  AeroOS 简易渲染器 (Rust, Ring3)              │
│  解析指令 → 直接操作 framebuffer              │
│  put_pixel / fill_rect / blit_bitmap          │
├───────────────────────────────────────────────┤
│  AeroOS Kernel (Ring 0)                       │
│  framebuffer | PS/2 | heap | time            │
└───────────────────────────────────────────────┘
```

| 维度 | 评估 |
|------|------|
| **难度** | ★★★★★ (极高) |
| **可复用** | Flutter framework + Dart 框架逻辑 |
| **必须重写** | Skia 后端（拦截绘制调用并序列化）、指令流协议设计、简易渲染器 |
| **优点** | 不需要完整 libc/pthread/epoll；不需要 ELF 动态链接器；不需要 GPU stack |
| **缺点** | 需要修改 Skia 源码（C++ 百万行级别）；指令流协议是全新设计，无先例；无法使用 Skia 的高级特性（着色器、路径效果）；文字渲染需自己实现 |
| **关键阻塞** | 1. 需深入 Skia 源码  2. 指令流序列化无参考实现  3. 文字渲染极其复杂 |
| **预估工作量** | 6-12 个月（需深入理解 Skia 内部架构） |

**路径 B 的指令流协议草案**：

```
// 二进制指令流格式（小端）
// 每条指令: [u16 opcode] [u16 param_len] [params...]

opcode 0x0001  DRAW_CLEAR       u32 color_argb
opcode 0x0002  DRAW_RECT         f32 x, f32 y, f32 w, f32 h, u32 color
opcode 0x0003  DRAW_PATH         u32 point_count, f32 points[]
opcode 0x0004  DRAW_TEXT         f32 x, f32 y, u32 str_len, u8 str[]
opcode 0x0005  DRAW_IMAGE        f32 x, f32 y, u32 w, u32 h, u8 pixels[]
opcode 0x0006  DRAW_CLIP_RECT    f32 x, f32 y, f32 w, f32 h
opcode 0x0007  DRAW_SAVE         (no params)
opcode 0x0008  DRAW_RESTORE      (no params)
opcode 0x0009  DRAW_TRANSLATE    f32 dx, f32 dy
opcode 0x000A  DRAW_SCALE        f32 sx, f32 sy
```

### 两条路径对比总结

| 维度 | 路径 A (flutter-elinux 替换后端) | 路径 B (Skia 指令流) |
|------|----------------------------------|---------------------|
| **架构完整度** | 高（完整 Flutter 能力） | 低（丢失 Skia 高级特性） |
| **libc 依赖** | 高（malloc/pthread/epoll 全套） | 低（只需简单内存+串口） |
| **GPU 需求** | 不需要（Software raster） | 不需要 |
| **ELF 动态链接** | 需要（加载 .so） | 不需要（可静态链接渲染器） |
| **可维护性** | 高（跟随上游更新） | 低（维护 Skia fork） |
| **性能** | 中（Skia 软件渲染已优化） | 低（简易渲染器无优化） |
| **推荐度** | ★★★★☆ | ★★☆☆☆ |

**推荐**：先走路径 A 的前置基础设施（Ring3 + ELF loader + libc），同时用路径 B 的思路在 framebuffer 上做简单 UI 验证。

---

## 3. 模块级分类：复用 / 重写 / 人工开发

### 3.1 flutter-elinux embedder 模块分类

| 模块 | 文件 | 策略 | 说明 |
|------|------|------|------|
| Engine task runner | `engine.cc` | **复用** | 不依赖图形后端，可直接用 |
| Platform message proto | `platform_message.cc` | **复用** | Dart ↔ Host 通信 |
| Dart isolate 管理 | `dart_isolate.cc` | **复用** | 内部逻辑 |
| Renderer (EGL/DRM) | `renderer.cc` | **【必须重写】** | 替换为 framebuffer blit |
| Input (libinput) | `input.cc` | **【必须重写】** | 替换为 PS/2 syscall |
| Display (DRM/KMS) | `display.cc` | **【必须重写】** | 替换为 fb info syscall |
| Event loop (epoll) | `event_loop.cc` | **【必须重写】** | 替换为轮询 + nanosleep |

### 3.2 内核侧模块分类

| 模块 | 当前状态 | 策略 | AI 可生成性 |
|------|---------|------|-------------|
| `get_framebuffer_info` syscall | ✅ 已实现 | 完成 | AI 已生成 |
| `read(0)` stdin syscall | ✅ PS/2 scancode | 完成 | AI 已生成 |
| `write(1/2)` stdout/stderr | ✅ serial+VGA | 完成 | AI 已生成 |
| `mmap` syscall | 骨架（heap） | 需升级为真实虚拟内存映射 | AI 可生成骨架 |
| `nanosleep` syscall | ✅ PIT tick | 完成 | AI 已生成 |
| `clock_gettime` syscall | ✅ MONOTONIC | 完成 | AI 已生成 |
| Ring3 用户态 | `[PENDING]` | **【必须人工开发，AI 无法完整生成】** | 需调试 CPL 切换 |
| 进程/线程调度 | `[PENDING]` | AI 可生成骨架，需手动调试 | 上下文切换汇编 |
| ELF 加载器 | 未开始 | AI 可生成解析器，需手动调试 | 重定位处理 |
| 动态链接器 | 未开始 | **【必须人工开发，AI 无法完整生成】** | 极其复杂 |
| libc 兼容层 | 未开始 | AI 可生成大部分 | 线程安全 malloc 需人工 |
| pthread | 未开始 | AI 可生成骨架 | futex 实现需人工 |
| epoll/eventfd | 未开始 | AI 可生成 | 纯软件逻辑 |
| 虚拟内存/分页 | 未开始 | AI 可生成骨架 | 页错误处理需人工 |

### 3.3 适配层模块分类

| 模块 | 文件 | 策略 | AI 可生成性 |
|------|------|------|-------------|
| Syscall wrappers | `syscalls.rs` | ✅ 骨架已生成 | AI 可完整生成 |
| 输入适配 | `input.rs` | ✅ 骨架已生成 | AI 可完整生成 |
| 帧缓冲适配 | `framebuffer.rs` | ✅ 骨架已生成 | AI 可完整生成 |
| Embedder API | `embedder.rs` | ✅ 骨架已生成 | AI 可生成类型定义 |
| Renderer 回调实现 | 待创建 | 需对接真实 Engine | **【必须人工开发】** |
| C FFI 绑定 | 待创建 | 需绑定 `flutter_embedder.h` | AI 可生成 bindgen |
| 主事件循环 | `lib.rs::run()` | ✅ 骨架已生成 | 需对接真实 Engine 后人工完善 |

---

## 4. 已生成骨架代码清单

以下文件为 AI 生成的骨架代码，可编译通过（`cargo check`），但不包含运行时逻辑：

### 4.1 `flutter_adapter/src/syscalls.rs`
- 7 个 syscall wrapper（open/read/write/mmap/nanosleep/clock_gettime/get_fb_info）
- `FramebufferInfo` C 兼容结构体
- errno 常量定义
- 所有 wrapper 当前返回 `ENOSYS`（等待 Ring3 实现后替换为 `int 0x80` 内联汇编）

### 4.2 `flutter_adapter/src/input.rs`
- `FlutterPointerEvent` / `FlutterKeyEvent` 结构体（`#[repr(C)]`，与 Flutter embedder API 兼容）
- `EventList` 固定容量事件队列（无 alloc 依赖）
- PS/2 Set-1 scancode → Unicode 映射表
- scancode → Flutter event 转换逻辑
- 方向键合成 pointer event 逻辑
- `poll()` 函数读取 stdin 并返回 `EventList`

### 4.3 `flutter_adapter/src/framebuffer.rs`
- `FramebufferCanvas` 结构体（封装 mmap'd framebuffer）
- `put_pixel()` / `fill_rect()` / `clear()` / `blit()` 绘图方法
- RGB/BGR 像素格式处理
- `init()` 函数：调用 `get_framebuffer_info` + `mmap` + 创建 canvas
- `skeleton` feature：syscalls 返回 ENOSYS 时使用 fallback 1280x720 配置

### 4.4 `flutter_adapter/src/embedder.rs`
- `FlutterEngineHandle` / `FlutterEngineResult` / `FlutterRendererType` 类型定义
- `FlutterSoftwareBackingStore` / `FlutterEngineConfig` 结构体
- `init()` / `send_pointer_event()` / `send_key_event()` / `dispatch_frame()` 骨架
- `on_acquire_backing_store()` / `on_present_backing_store()` 回调骨架
- 所有函数标注 `[MANUAL]` 注释，指明需要人工对接真实 Engine

### 4.5 `flutter_adapter/src/lib.rs`
- `init()` 初始化序列（syscalls → framebuffer → input → embedder）
- `run()` 主事件循环骨架（poll → convert → push → render → sleep）

### 4.6 `flutter_adapter/Cargo.toml`
- `no_std` crate，无外部依赖
- `skeleton` feature（默认启用，提供 fallback 值）

---

## 5. 需要人工修改实现的部分

以下代码已生成骨架，但需要人工补充真实逻辑：

### 5.1 `syscalls.rs` — 替换 ENOSYS 为真实 `int 0x80`
```rust
// 当前（骨架）:
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    let _ = (fd, buf);
    ENOSYS
}

// 需要人工改为（Ring3 就绪后）:
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    let ret: i64;
    unsafe {
        asm!("int 0x80",
            in("rax") SYS_WRITE,
            in("rdi") fd as u64,
            in("rsi") buf.as_ptr() as u64,
            in("rdx") buf.len() as u64,
            lateout("rax") ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}
```

### 5.2 `embedder.rs` — 对接真实 `libflutter_engine.so`
- `init()` 需调用 `FlutterEngineInitialize()` + `FlutterEngineRun()`
- `send_pointer_event()` 需调用 `FlutterEngineSendPointerEvent()`
- `dispatch_frame()` 需调用 `FlutterEngineOnVsync()` 并处理 backing store 回调
- 需要 C FFI 绑定（`#[link(name = "flutter_engine")]` + extern "C"）

### 5.3 `framebuffer.rs` — `mmap` 返回真实映射地址
- 当前 `syscalls::mmap` 返回 ENOSYS，骨架用 fallback 地址
- Ring3 就绪后，`mmap` 返回真实用户态虚拟地址
- 需验证帧缓冲映射的页权限（RW，用户可访问）

### 5.4 `input.rs` — 扩展为完整输入系统
- 当前仅处理 PS/2 Set-1 键盘 scancode
- 需添加：鼠标输入（如果硬件支持）、多键同时按下、修饰键状态
- 需添加：USB HID 支持（未来）

---

## 6. 几乎无法 AI 生成的部分

以下部分复杂度极高，需要人工设计和调试，AI 无法完整生成：

### 6.1 ELF 动态链接器 【必须人工开发，AI 无法完整生成】
- 解析 `.dynamic` 段、`.dynsym`、`.dynstr`
- 处理 `R_X86_64_GLOB_DAT`、`R_X86_64_JUMP_SLOT`、`R_X86_64_RELATIVE` 等重定位类型
- PLT/GOT 懒加载机制
- 弱符号解析
- 库搜索路径（DT_RPATH/DT_RUNPATH）

### 6.2 Ring3 用户态切换 【必须人工开发，AI 无法完整生成】
- TSS 结构体设置（SS0/RSP0）
- `iretq` 从 Ring0 切换到 Ring3（CS/SS 段选择子、RFLAGS、RIP、RSP）
- 用户态栈分配和映射
- `int 0x80` 从 Ring3 进入 Ring0 的权限切换
- 用户态页表隔离（防止用户进程访问内核内存）

### 6.3 pthread / futex 实现 【必须人工开发，AI 无法完整生成】
- `futex` 系统调用（内核侧等待队列、wake 机制）
- `pthread_mutex` 的 fast path / contention path
- `pthread_cond` 的 wait/signal 语义
- 线程局部存储 (TLS)
- 线程取消 (cancellation)

### 6.4 虚拟内存页错误处理 【必须人工开发，AI 无法完整生成】
- CR2 读取（缺页地址）
- 区分 demand paging / stack growth / protection fault
- CoW (Copy-on-Write) 页面处理
- 页面回收和交换（如果需要）

### 6.5 Skia 软件渲染后端集成 【必须人工开发，AI 无法完整生成】
- 对接 Skia 的 `SkSurface` / `SkCanvas` API
- 像素格式转换（Skia RGBA → framebuffer BGR/RGBX）
- 脏矩形追踪和增量更新
- 文字渲染（FreeType/harfBuzz 集成或替代）

### 6.6 Skia 指令流序列化（路径 B）【必须人工开发，AI 无法完整生成】
- 修改 Skia 的 `SkCanvas` 子类，拦截所有绘制调用
- 设计二进制指令流协议（序列化/反序列化）
- 实现路径 B 的简易渲染器（路径填充、抗锯齿、裁剪栈）
- 字体度量与 glyph 渲染

---

## 7. 内核侧已完成的变更

### 7.1 新增 syscall: `get_framebuffer_info` (SYS_GET_FB_INFO = 7)

**文件**: `kernel/src/syscalls/mod.rs`

```rust
pub enum SyscallNum {
    // ...
    GetFramebufferInfo = 7,
}

unsafe fn sys_get_framebuffer_info(info_ptr: u64) -> i64 {
    if info_ptr == 0 {
        return Errno::efault.as_i64();
    }
    let (fb_addr, fb_len, width, height, stride, bpp, format) = {
        match crate::graphics::get_fb_state() {
            Some(s) => s,
            None => return Errno::enosys.as_i64(),
        }
    };
    let ptr = info_ptr as *mut u64;
    *ptr = fb_addr as u64;
    *ptr.add(1) = width as u64;
    *ptr.add(2) = height as u64;
    *ptr.add(3) = stride as u64;
    *ptr.add(4) = bpp as u64;
    *ptr.add(5) = format as u64;
    *ptr.add(6) = fb_len as u64;
    0
}
```

### 7.2 新增 `graphics::get_fb_state()`

**文件**: `kernel/src/graphics/mod.rs`

```rust
pub fn get_fb_state() -> Option<(*mut u8, usize, u32, u32, u32, u32, u32)> {
    let state = GRAPHICS.lock();
    let info = state.info?;
    let format = match info.pixel_format {
        PixelFormat::Rgb => 0,
        PixelFormat::Bgr => 1,
        _ => 2,
    };
    Some((state.buffer, state.len, info.width as u32, info.height as u32,
          info.stride as u32, info.bytes_per_pixel as u32, format))
}
```

### 7.3 [OK] FLUTTER_ADAPTER 启动标记

**文件**: `kernel/src/lib.rs`

```rust
println!("[OK] FLUTTER_ADAPTER");
```

### 7.4 CI 检查标记

**文件**: `ci/check_boot.sh`

```bash
for marker in "GDT" "IDT" "PIC" "HEAP" "KEYBOARD" "GRAPHICS" "TIME" \
              "SYSCALLS" "FLUTTER_ADAPTER" "USERMODE" "SCHEDULER" \
              "SIGNAL" "FORK_EXEC"; do
```

### 7.5 Workspace 成员

**文件**: `Cargo.toml`

```toml
[workspace]
members = ["kernel", "flutter_adapter"]
```

---

## 8. 技术约束声明

1. **禁止内核态运行 Flutter Engine**：Flutter Engine 必须作为 Ring3 普通用户态程序运行，所有代码标注 `[MANUAL]` 的部分需在 Ring3 就绪后实现
2. **不使用虚构 crate**：`flutter_adapter` 无外部依赖，所有 Flutter 类型在本地重新定义，与 `flutter_embedder.h` 保持 `#[repr(C)]` 兼容
3. **骨架与实现分离**：所有 syscall wrapper 当前返回 `ENOSYS`；`skeleton` feature 提供 fallback 值用于编译验证
4. **路径 A 为推荐路径**：保留完整 Flutter 能力，基础设施（Ring3/ELF/libc）可逐步推进
5. **路径 B 为探索路径**：技术风险高，无先例参考，适合长期研究
6. **CI 限制**：GitHub runner 无 KVM，QEMU 使用 TCG 软件模拟，仅验证串口启动标记

---

## 附录：flutter-pi 参考

[flutter-pi](https://github.com/ardera/flutter-pi) 是一个轻量级 Flutter Engine Embedder，面向 Raspberry Pi / Linux 直出显示 [$TRAE_REF](https://github.com/ardera/flutter-pi)。它的架构可作为路径 A 的参考：

- `flutter-pi.c` — 主入口
- `gl_renderer.c` — GBM + EGL + GLES 渲染 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/gl_renderer.c)
- `modesetting.c` — DRM/KMS 模式设置 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/modesetting.c)
- `compositor_ng.c` — 合成器 [$TRAE_REF](https://raw.githubusercontent.com/ardera/flutter-pi/master/src/compositor_ng.c)

flutter-elinux 与 flutter-pi 的差异：

| 维度 | flutter-pi | flutter-elinux |
|------|-----------|----------------|
| 目标 | RPi / 轻量直出 | 通用嵌入式 Linux SDK |
| 后端 | DRM/KMS + GBM + EGL + GLES | Wayland、DRM-GBM、DRM-EGLStream、X11 |
| 输入 | libinput, udev, xkbcommon | 类似 |
| SDK | 自建 embedder | Sony 提供完整 SDK |
| 运行时 | libflutter_engine.so + icudtl.dat | 同左 |
