/// AeroOS Flutter desktop shell.
///
/// A minimal desktop environment that runs on top of the AeroOS kernel.
/// In the full build pipeline the Flutter engine embedder (flutter-pi or
/// flutter-elinux) attaches to the kernel's DRM/GBM framebuffer and renders
/// this Dart UI directly on the bare-metal display.
///
/// The shell provides:
/// - A taskbar with system info and a clock
/// - A desktop area with draggable windows
/// - A simple app launcher

import 'dart:async';
import 'dart:ui';

import 'package:flutter/material.dart';

void main() {
  runApp(const AeroShell());
}

/// Root widget: a fullscreen desktop with a taskbar and window manager.
class AeroShell extends StatelessWidget {
  const AeroShell({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'AeroOS',
      debugShowCheckedModeBanner: false,
      theme: _buildTheme(),
      home: const Desktop(),
    );
  }

  ThemeData _buildTheme() {
    return ThemeData(
      useMaterial3: true,
      colorScheme: ColorScheme.fromSeed(
        seedColor: const Color(0x3B82F6),
        brightness: Brightness.dark,
      ),
      fontFamily: 'Noto Sans CJK SC',
    );
  }
}

/// The desktop surface: gradient background, windows, and a taskbar.
class Desktop extends StatefulWidget {
  const Desktop({super.key});

  @override
  State<Desktop> createState() => _DesktopState();
}

class _DesktopState extends State<Desktop> {
  final List<_Window> _windows = [];
  final DateTime _bootTime = DateTime.now();
  TimeOfDay _currentTime = TimeOfDay.now();

  @override
  void initState() {
    super.initState();
    // Seed the desktop with a welcome window.
    _windows.add(_Window(
      title: 'Welcome',
      child: const _WelcomeContent(),
    ));
    // Tick the clock every second.
    Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted) setState(() => _currentTime = TimeOfDay.now());
    });
  }

  void _openApp(String name) {
    setState(() {
      _windows.add(_Window(
        title: name,
        child: _AppContent(appName: name),
      ));
    });
  }

  void _closeWindow(int index) {
    setState(() => _windows.removeAt(index));
  }

  String _formatUptime() {
    final elapsed = DateTime.now().difference(_bootTime);
    final m = elapsed.inMinutes;
    final s = elapsed.inSeconds % 60;
    return '${m}m ${s}s';
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Stack(
        children: [
          // Gradient desktop background.
          Container(
            decoration: const BoxDecoration(
              gradient: LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: [
                  Color(0xFF0B122A),
                  Color(0xFF050810),
                ],
              ),
            ),
          ),
          // Windows.
          ..._windows.asMap().entries.map((entry) {
            final i = entry.key;
            final w = entry.value;
            return Positioned(
              left: 80.0 + i * 40,
              top: 60.0 + i * 30,
              child: _DraggableWindow(
                window: w,
                onClose: () => _closeWindow(i),
              ),
            );
          }),
          // Taskbar (top).
          Positioned(
            top: 0,
            left: 0,
            right: 0,
            child: _Taskbar(
              onLaunch: _openApp,
              uptime: _formatUptime(),
              clock: _currentTime,
            ),
          ),
        ],
      ),
    );
  }
}

/// A draggable, closable window with a title bar.
class _DraggableWindow extends StatefulWidget {
  final _Window window;
  final VoidCallback onClose;

  const _DraggableWindow({
    required this.window,
    required this.onClose,
  });

  @override
  State<_DraggableWindow> createState() => _DraggableWindowState();
}

class _DraggableWindowState extends State<_DraggableWindow> {
  Offset _position = Offset.zero;

  @override
  Widget build(BuildContext context) {
    return Transform.translate(
      offset: _position,
      child: Container(
        width: 420,
        height: 300,
        decoration: BoxDecoration(
          color: const Color(0xFF0F1724).withValues(alpha: 0.95),
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: const Color(0xFF1D4E89), width: 1),
          boxShadow: const [
            BoxShadow(
              color: Color(0x44000000),
              blurRadius: 12,
              offset: Offset(2, 4),
            ),
          ],
        ),
        child: Column(
          children: [
            // Title bar (draggable).
            GestureDetector(
              onPanUpdate: (details) {
                setState(() => _position += details.delta);
              },
              child: Container(
                height: 32,
                padding: const EdgeInsets.symmetric(horizontal: 12),
                decoration: const BoxDecoration(
                  color: Color(0xFF1E293B),
                  borderRadius: BorderRadius.vertical(top: Radius.circular(8)),
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        widget.window.title,
                        style: const TextStyle(
                          color: Colors.white70,
                          fontSize: 13,
                          fontWeight: FontWeight.w500,
                        ),
                      ),
                    ),
                    GestureDetector(
                      onTap: widget.onClose,
                      child: const Icon(Icons.close, size: 16, color: Colors.redAccent),
                    ),
                  ],
                ),
              ),
            ),
            // Window body.
            Expanded(child: widget.window.child),
          ],
        ),
      ),
    );
  }
}

/// Top taskbar: logo, app buttons, uptime, clock.
class _Taskbar extends StatelessWidget {
  final void Function(String) onLaunch;
  final String uptime;
  final TimeOfDay clock;

  const _Taskbar({
    required this.onLaunch,
    required this.uptime,
    required this.clock,
  });

  String _fmtClock() {
    final h = clock.hour.toString().padLeft(2, '0');
    final m = clock.minute.toString().padLeft(2, '0');
    return '$h:$m';
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 36,
      decoration: BoxDecoration(
        color: const Color(0xE63B82F6),
        border: Border(
          bottom: BorderSide(color: const Color(0xFF1D4E89), width: 2),
        ),
      ),
      child: Row(
        children: [
          // AeroOS logo mark.
          Container(
            margin: const EdgeInsets.only(left: 8),
            width: 20,
            height: 20,
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(4),
            ),
            child: const Center(
              child: Text(
                'A',
                style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold),
              ),
            ),
          ),
          const SizedBox(width: 12),
          const Text(
            'AeroOS',
            style: TextStyle(
              color: Colors.white,
              fontSize: 13,
              fontWeight: FontWeight.bold,
            ),
          ),
          const SizedBox(width: 24),
          // App launcher buttons.
          _AppButton(icon: Icons.terminal, label: 'Terminal', onTap: () => onLaunch('Terminal')),
          const SizedBox(width: 8),
          _AppButton(icon: Icons.calculate, label: 'Calculator', onTap: () => onLaunch('Calculator')),
          const SizedBox(width: 8),
          _AppButton(icon: Icons.text_snippet, label: 'Notes', onTap: () => onLaunch('Notes')),
          const Spacer(),
          // System info.
          Text(
            'up ${uptime}',
            style: const TextStyle(color: Colors.white70, fontSize: 11),
          ),
          const SizedBox(width: 12),
          Text(
            _fmtClock(),
            style: const TextStyle(color: Colors.white, fontSize: 13, fontWeight: FontWeight.w500),
          ),
          const SizedBox(width: 12),
        ],
      ),
    );
  }
}

/// A single app-launcher button in the taskbar.
class _AppButton extends StatelessWidget {
  final IconData icon;
  final String label;
  final VoidCallback onTap;

  const _AppButton({required this.icon, required this.label, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(4),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 14, color: Colors.white70),
            const SizedBox(width: 4),
            Text(label, style: const TextStyle(color: Colors.white70, fontSize: 11)),
          ],
        ),
      ),
    );
  }
}

/// Placeholder content for launched apps.
class _AppContent extends StatelessWidget {
  final String appName;

  const _AppContent({required this.appName});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(_iconFor(appName), size: 48, color: const Color(0xFF3B82F6)),
            const SizedBox(height: 16),
            Text(
              appName,
              style: const TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 8),
            const Text(
              'This is a placeholder app in the AeroOS Flutter shell.\n'
              'On bare metal it would render on the kernel framebuffer.',
              textAlign: TextAlign.center,
              style: TextStyle(color: Colors.white54, fontSize: 12),
            ),
          ],
        ),
      ),
    );
  }

  IconData _iconFor(String name) {
    switch (name) {
      case 'Terminal':
        return Icons.terminal;
      case 'Calculator':
        return Icons.calculate;
      case 'Notes':
        return Icons.text_snippet;
      default:
        return Icons.apps;
    }
  }
}

/// Welcome window content shown on boot.
class _WelcomeContent extends StatelessWidget {
  const _WelcomeContent();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'Welcome to AeroOS',
            style: TextStyle(fontSize: 20, fontWeight: FontWeight.bold, color: Color(0xFF3B82F6)),
          ),
          const SizedBox(height: 12),
          Text(
            'A minimal operating system built with Rust runtime and Flutter.\n\n'
            'Architecture:\n'
            '  • Kernel: Rust freestanding x86_64 (bootloader 0.11)\n'
            '  • Shell: Flutter desktop (Material 3)\n'
            '  • Graphics: Linear framebuffer / DRM-GBM-EGL\n\n'
            'Click the app buttons in the taskbar to open windows.',
            style: TextStyle(color: Colors.white70, fontSize: 12, height: 1.6),
          ),
        ],
      ),
    );
  }
}

/// Internal data model for a desktop window.
class _Window {
  final String title;
  final Widget child;

  _Window({required this.title, required this.child});
}
