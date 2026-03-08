import 'dart:math';
import 'dart:async';
import 'package:flutter/material.dart';
import '../models/threat_event.dart';
import '../models/iot_device.dart';
import '../models/drone_patrol.dart';
import '../services/agent_stream_service.dart';

/// Interactive 3D globe visualization built with CustomPainter.
/// Renders an orthographic-projection Earth sphere with:
///  - Threat markers (colour-coded by severity)
///  - IoT device location dots
///  - Drone patrol coverage zones
///  - Network connection arcs
class GlobeView extends StatefulWidget {
  const GlobeView({super.key});

  @override
  State<GlobeView> createState() => _GlobeViewState();
}

class _GlobeViewState extends State<GlobeView> with TickerProviderStateMixin {
  late AnimationController _rotationController;
  late AnimationController _pulseController;
  late StreamSubscription<ThreatEvent> _threatSub;

  double _rotationOffset = 0;
  Offset? _dragStart;
  double _dragStartOffset = 0;

  final List<ThreatEvent> _threats = [];
  final _maxThreats = 30;

  // Mock IoT devices
  final List<IotDevice> _devices = _mockDevices();
  // Mock drone patrols
  final List<DronePatrol> _drones = _mockDrones();

  @override
  void initState() {
    super.initState();
    _rotationController = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 60),
    )..repeat();

    _pulseController = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 2),
    )..repeat(reverse: true);

    _threatSub = AgentStreamService().threatHunterStream().listen((event) {
      if (!mounted) return;
      setState(() {
        _threats.insert(0, event);
        if (_threats.length > _maxThreats) _threats.removeLast();
      });
    });
  }

  @override
  void dispose() {
    _rotationController.dispose();
    _pulseController.dispose();
    _threatSub.cancel();
    super.dispose();
  }

  void _onPanStart(DragStartDetails d) {
    _dragStart = d.localPosition;
    _dragStartOffset = _rotationOffset;
    _rotationController.stop();
  }

  void _onPanUpdate(DragUpdateDetails d) {
    if (_dragStart == null) return;
    setState(() {
      _rotationOffset =
          _dragStartOffset + (d.localPosition.dx - _dragStart!.dx) / 200;
    });
  }

  void _onPanEnd(DragEndDetails d) {
    _dragStart = null;
    _rotationController.repeat();
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Expanded(
          flex: 3,
          child: GestureDetector(
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: _onPanEnd,
            child: AnimatedBuilder(
              animation: Listenable.merge([_rotationController, _pulseController]),
              builder: (context, _) {
                final autoRot =
                    _rotationController.value * 2 * pi + _rotationOffset;
                return CustomPaint(
                  painter: GlobePainter(
                    rotation: autoRot,
                    pulse: _pulseController.value,
                    threats: _threats,
                    devices: _devices,
                    drones: _drones,
                  ),
                  child: const SizedBox.expand(),
                );
              },
            ),
          ),
        ),
        // Legend
        Container(
          color: Colors.black87,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceAround,
            children: [
              _legend(Colors.red, 'Critical'),
              _legend(Colors.orange, 'Medium'),
              _legend(Colors.greenAccent, 'Anomaly'),
              _legend(Colors.cyanAccent, 'IoT Device'),
              _legend(Colors.deepOrange.withAlpha(128), 'Drone Zone'),
            ],
          ),
        ),
        // Recent threats list
        Expanded(
          flex: 1,
          child: ListView.builder(
            itemCount: _threats.length,
            itemBuilder: (ctx, i) {
              final t = _threats[i];
              return ListTile(
                dense: true,
                leading: Icon(Icons.warning_amber,
                    color: _severityColor(t.severity), size: 16),
                title: Text(t.description,
                    style: const TextStyle(fontSize: 12),
                    overflow: TextOverflow.ellipsis),
                subtitle: Text(
                    '${t.agentName} · ${_fmtTime(t.timestamp)}',
                    style: TextStyle(fontSize: 10, color: Colors.grey[500])),
                trailing: Text(
                    '${(t.confidence * 100).toStringAsFixed(0)}%',
                    style: TextStyle(
                        color: _severityColor(t.severity), fontSize: 11)),
              );
            },
          ),
        ),
      ],
    );
  }

  Widget _legend(Color color, String label) => Row(
        children: [
          Container(width: 10, height: 10, decoration: BoxDecoration(color: color, shape: BoxShape.circle)),
          const SizedBox(width: 4),
          Text(label, style: const TextStyle(fontSize: 10, color: Colors.white70)),
        ],
      );

  Color _severityColor(ThreatSeverity s) {
    switch (s) {
      case ThreatSeverity.critical:
        return Colors.red;
      case ThreatSeverity.medium:
        return Colors.orange;
      case ThreatSeverity.anomaly:
        return Colors.greenAccent;
    }
  }

  String _fmtTime(DateTime t) =>
      '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}:${t.second.toString().padLeft(2, '0')}';
}

// ── Globe Painter ───────────────────────────────────────────────────────────

class GlobePainter extends CustomPainter {
  final double rotation;
  final double pulse;
  final List<ThreatEvent> threats;
  final List<IotDevice> devices;
  final List<DronePatrol> drones;

  GlobePainter({
    required this.rotation,
    required this.pulse,
    required this.threats,
    required this.devices,
    required this.drones,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final cx = size.width / 2;
    final cy = size.height / 2;
    final radius = min(cx, cy) * 0.85;

    _drawGlobe(canvas, Offset(cx, cy), radius);
    _drawGraticule(canvas, Offset(cx, cy), radius);
    _drawDroneZones(canvas, Offset(cx, cy), radius);
    _drawNetworkArcs(canvas, Offset(cx, cy), radius);
    _drawDevices(canvas, Offset(cx, cy), radius);
    _drawThreats(canvas, Offset(cx, cy), radius);
  }

  void _drawGlobe(Canvas canvas, Offset center, double r) {
    // Atmosphere glow
    final glowPaint = Paint()
      ..shader = RadialGradient(
        colors: [
          Colors.blue.withAlpha(26),
          Colors.blue.withAlpha(13),
          Colors.transparent,
        ],
        stops: const [0.82, 0.92, 1.0],
      ).createShader(Rect.fromCircle(center: center, radius: r * 1.15));
    canvas.drawCircle(center, r * 1.15, glowPaint);

    // Ocean
    final oceanPaint = Paint()
      ..shader = RadialGradient(
        colors: [const Color(0xFF0D2137), const Color(0xFF0A1A2F)],
        center: const Alignment(-0.3, -0.3),
      ).createShader(Rect.fromCircle(center: center, radius: r));
    canvas.drawCircle(center, r, oceanPaint);

    // Simplified continents as arcs
    _drawContinents(canvas, center, r);

    // Sphere highlight
    final highlightPaint = Paint()
      ..shader = RadialGradient(
        colors: [Colors.white.withAlpha(51), Colors.transparent],
        center: const Alignment(-0.5, -0.5),
      ).createShader(Rect.fromCircle(center: center, radius: r));
    canvas.drawCircle(center, r, highlightPaint);

    // Border
    canvas.drawCircle(center, r,
        Paint()
          ..color = Colors.blue.withAlpha(77)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5);
  }

  void _drawContinents(Canvas canvas, Offset center, double r) {
    final paint = Paint()..color = const Color(0xFF1A3A1A);
    for (final blob in _continentPolygons) {
      final path = Path();
      bool first = true;
      for (final (lon, lat) in blob) {
        final pt = _project(lon, lat, center, r);
        if (pt == null) continue;
        if (first) {
          path.moveTo(pt.dx, pt.dy);
          first = false;
        } else {
          path.lineTo(pt.dx, pt.dy);
        }
      }
      path.close();
      canvas.drawPath(path, paint);
    }
  }

  void _drawGraticule(Canvas canvas, Offset center, double r) {
    final paint = Paint()
      ..color = Colors.blue.withAlpha(31)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 0.5;

    // Latitude lines
    for (final lat in [-60.0, -30.0, 0.0, 30.0, 60.0]) {
      _drawLatitudeLine(canvas, center, r, lat, paint);
    }
    // Longitude lines
    for (var lon = -180.0; lon < 180; lon += 30) {
      _drawLongitudeLine(canvas, center, r, lon, paint);
    }
  }

  void _drawLatitudeLine(Canvas canvas, Offset center, double r, double lat, Paint paint) {
    final path = Path();
    bool started = false;
    for (var lon = -180.0; lon <= 180; lon += 5) {
      final pt = _project(lon, lat, center, r);
      if (pt == null) continue;
      if (!started) { path.moveTo(pt.dx, pt.dy); started = true; }
      else { path.lineTo(pt.dx, pt.dy); }
    }
    canvas.drawPath(path, paint);
  }

  void _drawLongitudeLine(Canvas canvas, Offset center, double r, double lon, Paint paint) {
    final path = Path();
    bool started = false;
    for (var lat = -90.0; lat <= 90; lat += 5) {
      final pt = _project(lon, lat, center, r);
      if (pt == null) continue;
      if (!started) { path.moveTo(pt.dx, pt.dy); started = true; }
      else { path.lineTo(pt.dx, pt.dy); }
    }
    canvas.drawPath(path, paint);
  }

  void _drawDroneZones(Canvas canvas, Offset center, double r) {
    for (final drone in drones) {
      if (!drone.active) continue;
      final pt = _project(drone.longitude, drone.latitude, center, r);
      if (pt == null) continue;
      final zonePaint = Paint()
        ..color = Colors.deepOrange.withAlpha(38)
        ..style = PaintingStyle.fill;
      final borderPaint = Paint()
        ..color = Colors.deepOrange.withAlpha(102)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1;
      // Coverage radius in screen pixels (approx)
      final radiusPx = drone.coverageRadiusDeg * r / 90;
      canvas.drawCircle(pt, radiusPx, zonePaint);
      canvas.drawCircle(pt, radiusPx, borderPaint);

      // Drone icon
      _drawText(canvas, '✈', pt.translate(-6, -8), 14, Colors.deepOrange);
    }
  }

  void _drawNetworkArcs(Canvas canvas, Offset center, double r) {
    if (devices.length < 2) return;
    final arcPaint = Paint()
      ..color = Colors.cyan.withAlpha(51)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 0.8;

    for (var i = 0; i < min(devices.length - 1, 5); i++) {
      final a = _project(devices[i].longitude, devices[i].latitude, center, r);
      final b = _project(devices[i + 1].longitude, devices[i + 1].latitude, center, r);
      if (a == null || b == null) continue;
      final mid = Offset((a.dx + b.dx) / 2, (a.dy + b.dy) / 2);
      final dist = (b - a).distance;
      final ctrl = mid.translate(0, -dist * 0.25);
      final path = Path()
        ..moveTo(a.dx, a.dy)
        ..quadraticBezierTo(ctrl.dx, ctrl.dy, b.dx, b.dy);
      canvas.drawPath(path, arcPaint);
    }
  }

  void _drawDevices(Canvas canvas, Offset center, double r) {
    for (final device in devices) {
      final pt = _project(device.longitude, device.latitude, center, r);
      if (pt == null) continue;
      final pulseR = 6 + pulse * 4;
      canvas.drawCircle(pt, pulseR,
          Paint()..color = Colors.cyanAccent.withAlpha((pulse * 80).toInt()));
      canvas.drawCircle(pt, 4, Paint()..color = Colors.cyanAccent);
    }
  }

  void _drawThreats(Canvas canvas, Offset center, double r) {
    for (final threat in threats) {
      final pt = _project(threat.longitude, threat.latitude, center, r);
      if (pt == null) continue;
      final color = _severityColor(threat.severity);
      // Outer pulse ring
      final pulseR = 8 + pulse * 6;
      canvas.drawCircle(pt, pulseR,
          Paint()..color = color.withAlpha((pulse * 60).toInt()));
      // Inner dot
      canvas.drawCircle(pt, 5, Paint()..color = color);
      // Severity icon
      final icon = threat.severity == ThreatSeverity.critical
          ? '🔴'
          : threat.severity == ThreatSeverity.medium
              ? '🟡'
              : '🟢';
      _drawText(canvas, icon, pt.translate(-8, -20), 12, color);
    }
  }

  /// Orthographic projection: lon/lat → canvas point (null if on back hemisphere).
  Offset? _project(double lon, double lat, Offset center, double r) {
    final lonRad = (lon * pi / 180) + rotation;
    final latRad = lat * pi / 180;
    final x = cos(latRad) * sin(lonRad);
    final y = -sin(latRad);
    final z = cos(latRad) * cos(lonRad);
    if (z < 0) return null; // back face
    return Offset(center.dx + x * r, center.dy + y * r);
  }

  Color _severityColor(ThreatSeverity s) {
    switch (s) {
      case ThreatSeverity.critical:
        return Colors.red;
      case ThreatSeverity.medium:
        return Colors.orange;
      case ThreatSeverity.anomaly:
        return Colors.greenAccent;
    }
  }

  void _drawText(Canvas canvas, String text, Offset offset, double size, Color color) {
    final tp = TextPainter(
      text: TextSpan(text: text, style: TextStyle(fontSize: size, color: color)),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, offset);
  }

  @override
  bool shouldRepaint(GlobePainter old) =>
      old.rotation != rotation || old.pulse != pulse || old.threats != threats;
}

// ── Continent polygon data ────────────────────────────────────────────────────
// Each entry is a list of (longitude, latitude) pairs forming a simplified
// convex polygon approximation of a continent's outline.

const _northAmericaPolygon = [
  (-120.0, 40.0), (-100.0, 50.0), (-80.0, 45.0),
  (-75.0, 30.0), (-90.0, 25.0), (-110.0, 30.0),
];

const _europePolygon = [
  (-5.0, 43.0), (0.0, 55.0), (20.0, 55.0),
  (30.0, 45.0), (15.0, 38.0),
];

const _asiaPolygon = [
  (60.0, 35.0), (60.0, 55.0), (140.0, 55.0),
  (145.0, 40.0), (110.0, 20.0), (80.0, 20.0),
];

const _africaPolygon = [
  (10.0, 0.0), (15.0, 15.0), (40.0, 10.0),
  (45.0, -10.0), (30.0, -35.0), (18.0, -35.0),
];

const _southAmericaPolygon = [
  (-75.0, 0.0), (-60.0, 5.0), (-40.0, -5.0),
  (-40.0, -20.0), (-55.0, -35.0), (-70.0, -20.0),
];

const _australiaPolygon = [
  (115.0, -20.0), (150.0, -20.0),
  (150.0, -38.0), (130.0, -38.0),
];

/// Simplified continent polygon data used by [GlobePainter._drawContinents].
const _continentPolygons = [
  _northAmericaPolygon,
  _europePolygon,
  _asiaPolygon,
  _africaPolygon,
  _southAmericaPolygon,
  _australiaPolygon,
];

// ── Mock data ────────────────────────────────────────────────────────────────

List<IotDevice> _mockDevices() => [
      IotDevice(id: 'd1', ip: '192.168.1.1', label: 'Home Router', type: DeviceType.iot,
          license: LicenseStatus.free, protectionEnabled: true, latitude: 40.7, longitude: -74.0,
          lastSeen: DateTime.now(), blockedAttempts: 3),
      IotDevice(id: 'd2', ip: '192.168.1.5', label: 'Smart Camera', type: DeviceType.iot,
          license: LicenseStatus.paid, protectionEnabled: true, latitude: 51.5, longitude: -0.1,
          lastSeen: DateTime.now(), blockedAttempts: 12),
      IotDevice(id: 'd3', ip: '10.0.0.2', label: 'Laptop', type: DeviceType.desktop,
          license: LicenseStatus.paid, protectionEnabled: true, latitude: 35.7, longitude: 139.7,
          lastSeen: DateTime.now(), blockedAttempts: 0),
      IotDevice(id: 'd4', ip: '10.0.0.8', label: 'Phone', type: DeviceType.mobile,
          license: LicenseStatus.unlicensed, protectionEnabled: false, latitude: 48.9, longitude: 2.3,
          lastSeen: DateTime.now(), blockedAttempts: 0),
    ];

List<DronePatrol> _mockDrones() => [
      DronePatrol(id: 'drone1', label: 'Alpha', latitude: 40.0, longitude: -70.0,
          altitudeKm: 0.5, coverageRadiusDeg: 8, active: true,
          path: [PatrolWaypoint(latitude: 40.0, longitude: -70.0), PatrolWaypoint(latitude: 45.0, longitude: -65.0)]),
      DronePatrol(id: 'drone2', label: 'Beta', latitude: 50.0, longitude: 10.0,
          altitudeKm: 0.5, coverageRadiusDeg: 6, active: true,
          path: [PatrolWaypoint(latitude: 50.0, longitude: 10.0), PatrolWaypoint(latitude: 48.0, longitude: 16.0)]),
    ];
