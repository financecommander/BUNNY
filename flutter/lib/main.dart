import 'package:flutter/material.dart';
import 'screens/globe_view.dart';
import 'screens/threat_dashboard.dart';
import 'screens/device_manager.dart';
import 'screens/agent_status_screen.dart';

void main() => runApp(const BunnyApp());

class BunnyApp extends StatelessWidget {
  const BunnyApp({super.key});

  @override
  Widget build(BuildContext context) => MaterialApp(
        title: '🐰 Bunny',
        theme: ThemeData.dark().copyWith(
          colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepOrange),
          appBarTheme: const AppBarTheme(
            backgroundColor: Color(0xFF0D0D0D),
            foregroundColor: Colors.deepOrangeAccent,
          ),
          scaffoldBackgroundColor: const Color(0xFF0D0D0D),
          bottomNavigationBarTheme: const BottomNavigationBarThemeData(
            backgroundColor: Color(0xFF121212),
            selectedItemColor: Colors.deepOrangeAccent,
            unselectedItemColor: Colors.white38,
          ),
        ),
        home: const HomeScreen(),
      );
}

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  int _selectedIndex = 0;

  static const _screens = [
    GlobeView(),
    ThreatDashboard(),
    DeviceManagerScreen(),
    AgentStatusScreen(),
  ];

  static const _tabs = [
    BottomNavigationBarItem(icon: Icon(Icons.public), label: 'Globe View'),
    BottomNavigationBarItem(icon: Icon(Icons.bar_chart), label: 'Threats'),
    BottomNavigationBarItem(icon: Icon(Icons.devices), label: 'Devices'),
    BottomNavigationBarItem(icon: Icon(Icons.smart_toy), label: 'Agents'),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('🐰 Bunny Defender'),
        actions: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Center(
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  color: Colors.green.withAlpha(40),
                  borderRadius: BorderRadius.circular(6),
                  border: Border.all(color: Colors.greenAccent.withAlpha(100)),
                ),
                child: const Text('● LIVE',
                    style: TextStyle(
                        color: Colors.greenAccent,
                        fontSize: 11,
                        fontWeight: FontWeight.bold)),
              ),
            ),
          ),
        ],
      ),
      body: IndexedStack(
        index: _selectedIndex,
        children: _screens,
      ),
      bottomNavigationBar: BottomNavigationBar(
        currentIndex: _selectedIndex,
        onTap: (i) => setState(() => _selectedIndex = i),
        type: BottomNavigationBarType.fixed,
        items: _tabs,
      ),
    );
  }
}
