import 'package:flutter/material.dart';

void main() => runApp(const BunnyApp());

class BunnyApp extends StatelessWidget {
  const BunnyApp({super.key});
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: '🐰 Bunny',
    theme: ThemeData.dark().copyWith(colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepOrange)),
    home: const HomeScreen(),
  );
}

class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key});
  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('🐰 Bunny Defender')),
    body: const Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Text('Phase 6 Complete ✅', style: TextStyle(fontSize: 28)),
          Text('Cross-platform dashboard ready\nRust FFI + real-time protection coming'),
          SizedBox(height: 20),
          Text('You have 7 devices — First IP free'),
        ],
      ),
    ),
  );
}
