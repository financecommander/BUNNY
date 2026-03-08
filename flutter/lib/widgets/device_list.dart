import 'package:flutter/material.dart';
import '../models/iot_device.dart';

/// Scrollable list of IoT/network devices with protection toggle.
class DeviceList extends StatelessWidget {
  final List<IotDevice> devices;
  final void Function(int index) onToggle;

  const DeviceList({super.key, required this.devices, required this.onToggle});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        for (var i = 0; i < devices.length; i++)
          _DeviceRow(device: devices[i], onToggle: () => onToggle(i)),
      ],
    );
  }
}

class _DeviceRow extends StatelessWidget {
  final IotDevice device;
  final VoidCallback onToggle;

  const _DeviceRow({required this.device, required this.onToggle});

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(bottom: 8),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: Colors.grey[900],
        borderRadius: BorderRadius.circular(10),
        border: Border.all(
            color: device.protectionEnabled
                ? Colors.greenAccent.withAlpha(80)
                : Colors.grey.withAlpha(60)),
      ),
      child: Row(
        children: [
          Text(_typeIcon(device.type), style: const TextStyle(fontSize: 22)),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Text(device.label,
                        style: const TextStyle(
                            fontWeight: FontWeight.bold, fontSize: 13)),
                    const SizedBox(width: 6),
                    _licenseBadge(device.license),
                  ],
                ),
                Text(device.ip,
                    style: const TextStyle(color: Colors.white54, fontSize: 11)),
                if (device.blockedAttempts > 0)
                  Text('${device.blockedAttempts} blocked attempts',
                      style: const TextStyle(color: Colors.redAccent, fontSize: 10)),
              ],
            ),
          ),
          Switch(
            value: device.protectionEnabled,
            activeColor: Colors.greenAccent,
            onChanged: (_) => onToggle(),
          ),
        ],
      ),
    );
  }

  Widget _licenseBadge(LicenseStatus status) {
    switch (status) {
      case LicenseStatus.free:
        return _badge('FREE ✅', Colors.greenAccent);
      case LicenseStatus.paid:
        return _badge('\$1/mo ✅', Colors.deepOrangeAccent);
      case LicenseStatus.unlicensed:
        return _badge('UNLOCK', Colors.grey);
    }
  }

  Widget _badge(String text, Color color) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
        decoration: BoxDecoration(
            color: color.withAlpha(30), borderRadius: BorderRadius.circular(4)),
        child: Text(text,
            style: TextStyle(
                fontSize: 9, color: color, fontWeight: FontWeight.bold)),
      );

  String _typeIcon(DeviceType type) {
    switch (type) {
      case DeviceType.iot:
        return '📡';
      case DeviceType.mobile:
        return '📱';
      case DeviceType.desktop:
        return '💻';
      case DeviceType.unknown:
        return '❓';
    }
  }
}
