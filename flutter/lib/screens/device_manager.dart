import 'package:flutter/material.dart';
import '../models/iot_device.dart';
import '../widgets/device_list.dart';
import '../widgets/license_manager.dart';

/// Device Manager screen: shows IP list with licensing control.
class DeviceManagerScreen extends StatefulWidget {
  const DeviceManagerScreen({super.key});

  @override
  State<DeviceManagerScreen> createState() => _DeviceManagerScreenState();
}

class _DeviceManagerScreenState extends State<DeviceManagerScreen> {
  // Mutable device list – toggle protection & license
  final List<IotDevice> _devices = [
    IotDevice(id: 'd1', ip: '192.168.1.1', label: 'Home Router', type: DeviceType.iot,
        license: LicenseStatus.free, protectionEnabled: true,
        latitude: 40.7, longitude: -74.0, lastSeen: DateTime.now(), blockedAttempts: 3),
    IotDevice(id: 'd2', ip: '192.168.1.5', label: 'Smart Camera', type: DeviceType.iot,
        license: LicenseStatus.paid, protectionEnabled: true,
        latitude: 51.5, longitude: -0.1, lastSeen: DateTime.now(), blockedAttempts: 12),
    IotDevice(id: 'd3', ip: '10.0.0.2', label: 'Laptop', type: DeviceType.desktop,
        license: LicenseStatus.paid, protectionEnabled: true,
        latitude: 35.7, longitude: 139.7, lastSeen: DateTime.now(), blockedAttempts: 0),
    IotDevice(id: 'd4', ip: '10.0.0.8', label: 'Phone', type: DeviceType.mobile,
        license: LicenseStatus.unlicensed, protectionEnabled: false,
        latitude: 48.9, longitude: 2.3, lastSeen: DateTime.now(), blockedAttempts: 0),
    IotDevice(id: 'd5', ip: '10.0.0.15', label: 'Smart TV', type: DeviceType.iot,
        license: LicenseStatus.unlicensed, protectionEnabled: false,
        latitude: 55.8, longitude: 37.6, lastSeen: DateTime.now(), blockedAttempts: 7),
    IotDevice(id: 'd6', ip: '10.0.0.22', label: 'Tablet', type: DeviceType.mobile,
        license: LicenseStatus.unlicensed, protectionEnabled: false,
        latitude: 19.1, longitude: 72.9, lastSeen: DateTime.now(), blockedAttempts: 1),
    IotDevice(id: 'd7', ip: '10.0.0.30', label: 'NAS Drive', type: DeviceType.desktop,
        license: LicenseStatus.unlicensed, protectionEnabled: false,
        latitude: 1.3, longitude: 103.8, lastSeen: DateTime.now(), blockedAttempts: 0),
  ];

  void _toggleProtection(int index) {
    setState(() {
      final d = _devices[index];
      if (!d.protectionEnabled && d.license != LicenseStatus.free) {
        // Prompt to upgrade for non-free devices
        _showUpgradeDialog(index);
        return;
      }
      _devices[index] = d.copyWith(protectionEnabled: !d.protectionEnabled);
    });
  }

  void _showUpgradeDialog(int index) {
    showDialog<void>(
      context: context,
      builder: (_) => AlertDialog(
        title: const Text('Upgrade Required'),
        content: Text(
            'Enable protection for ${_devices[index].label} for \$1/month.\n\nFirst IP is always free!'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
          ElevatedButton(
            style: ElevatedButton.styleFrom(backgroundColor: Colors.deepOrange),
            onPressed: () {
              Navigator.pop(context);
              setState(() {
                _devices[index] = _devices[index].copyWith(
                    protectionEnabled: true, license: LicenseStatus.paid);
              });
            },
            child: const Text('Subscribe \$1/mo'),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          LicenseManager(devices: _devices),
          const SizedBox(height: 16),
          const Text('🔒 Protected Devices',
              style: TextStyle(
                  fontSize: 16,
                  fontWeight: FontWeight.bold,
                  color: Colors.deepOrangeAccent)),
          const SizedBox(height: 8),
          DeviceList(devices: _devices, onToggle: _toggleProtection),
        ],
      ),
    );
  }
}
