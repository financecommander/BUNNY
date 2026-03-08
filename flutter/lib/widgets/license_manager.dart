import 'package:flutter/material.dart';
import '../models/iot_device.dart';

/// Visual IP licensing overview with "First IP Free" badge.
class LicenseManager extends StatelessWidget {
  final List<IotDevice> devices;

  const LicenseManager({super.key, required this.devices});

  @override
  Widget build(BuildContext context) {
    final totalDevices = devices.length;
    final protectedCount = devices.where((d) => d.protectionEnabled).length;
    final paidCount =
        devices.where((d) => d.license == LicenseStatus.paid).length;
    final monthlyCost = paidCount; // $1 per paid IP

    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        gradient: LinearGradient(
          colors: [Colors.deepOrange.withAlpha(40), Colors.black],
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
        ),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: Colors.deepOrange.withAlpha(100)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Text('🐰', style: TextStyle(fontSize: 28)),
              const SizedBox(width: 8),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('BUNNY Protection',
                      style: TextStyle(
                          fontSize: 16,
                          fontWeight: FontWeight.bold,
                          color: Colors.deepOrangeAccent)),
                  Text('$totalDevices devices detected',
                      style:
                          const TextStyle(color: Colors.white60, fontSize: 12)),
                ],
              ),
            ],
          ),
          const SizedBox(height: 12),
          // First IP free badge
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
            decoration: BoxDecoration(
                color: Colors.green.withAlpha(30),
                borderRadius: BorderRadius.circular(8),
                border: Border.all(color: Colors.greenAccent.withAlpha(100))),
            child: const Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(Icons.check_circle, color: Colors.greenAccent, size: 16),
                SizedBox(width: 6),
                Text('First IP Free — Protected Forever ✅',
                    style: TextStyle(color: Colors.greenAccent, fontSize: 12)),
              ],
            ),
          ),
          const SizedBox(height: 10),
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              _stat('Protected', '$protectedCount', Colors.greenAccent),
              _stat('Paid IPs', '$paidCount', Colors.deepOrangeAccent),
              _stat('Monthly Cost', '\$$monthlyCost/mo', Colors.cyanAccent),
              _stat('Unprotected', '${totalDevices - protectedCount}', Colors.redAccent),
            ],
          ),
        ],
      ),
    );
  }

  Widget _stat(String label, String value, Color color) => Column(
        children: [
          Text(value,
              style: TextStyle(
                  color: color, fontSize: 20, fontWeight: FontWeight.bold)),
          Text(label,
              style: const TextStyle(color: Colors.white54, fontSize: 10)),
        ],
      );
}
