import 'dart:async';
import 'dart:math';
import 'package:flutter/material.dart';
import '../models/threat_event.dart';
import '../services/agent_stream_service.dart';

/// 2D heatmap grid showing threat intensity per geographic region.
/// Updates every 5 seconds.
class ThreatHeatmap extends StatefulWidget {
  const ThreatHeatmap({super.key});

  @override
  State<ThreatHeatmap> createState() => _ThreatHeatmapState();
}

class _ThreatHeatmapState extends State<ThreatHeatmap> {
  // 12 × 6 grid (longitude × latitude buckets)
  static const _cols = 12;
  static const _rows = 6;
  final _grid = List.generate(_rows, (_) => List<double>.filled(_cols, 0));

  late StreamSubscription<ThreatEvent> _sub;
  Timer? _decayTimer;

  @override
  void initState() {
    super.initState();
    _sub = AgentStreamService().threatHunterStream().listen(_onThreat);
    // Decay heat every 5 s
    _decayTimer = Timer.periodic(const Duration(seconds: 5), (_) {
      if (!mounted) return;
      setState(() {
        for (var r = 0; r < _rows; r++) {
          for (var c = 0; c < _cols; c++) {
            _grid[r][c] = max(0, _grid[r][c] - 0.1);
          }
        }
      });
    });
  }

  void _onThreat(ThreatEvent e) {
    // Map lat/lon to grid cell
    final col = ((e.longitude + 180) / 360 * _cols).clamp(0, _cols - 1).toInt();
    final row = ((90 - e.latitude) / 180 * _rows).clamp(0, _rows - 1).toInt();
    if (!mounted) return;
    setState(() {
      final heat = e.severity == ThreatSeverity.critical
          ? 0.5
          : e.severity == ThreatSeverity.medium
              ? 0.3
              : 0.15;
      _grid[row][col] = min(1.0, _grid[row][col] + heat);
    });
  }

  @override
  void dispose() {
    _sub.cancel();
    _decayTimer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AspectRatio(
      aspectRatio: 2.0,
      child: Container(
        decoration: BoxDecoration(
          color: Colors.black,
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: Colors.deepOrange.withAlpha(80)),
        ),
        child: CustomPaint(
          painter: _HeatmapPainter(_grid),
        ),
      ),
    );
  }
}

class _HeatmapPainter extends CustomPainter {
  final List<List<double>> grid;
  _HeatmapPainter(this.grid);

  @override
  void paint(Canvas canvas, Size size) {
    final rows = grid.length;
    final cols = grid[0].length;
    final cellW = size.width / cols;
    final cellH = size.height / rows;

    for (var r = 0; r < rows; r++) {
      for (var c = 0; c < cols; c++) {
        final v = grid[r][c];
        if (v <= 0) continue;
        final color = _heatColor(v);
        canvas.drawRect(
          Rect.fromLTWH(c * cellW, r * cellH, cellW, cellH),
          Paint()..color = color,
        );
      }
    }

    // Grid lines
    final linePaint = Paint()
      ..color = Colors.white12
      ..strokeWidth = 0.5;
    for (var c = 0; c <= cols; c++) {
      canvas.drawLine(Offset(c * cellW, 0), Offset(c * cellW, size.height), linePaint);
    }
    for (var r = 0; r <= rows; r++) {
      canvas.drawLine(Offset(0, r * cellH), Offset(size.width, r * cellH), linePaint);
    }
  }

  static const _lowThreshold = 0.33;
  static const _mediumThreshold = 0.66;
  static const _lowMaxAlpha = 160.0;
  static const _mediumBaseAlpha = 80.0;
  static const _highBaseAlpha = 160.0;
  static const _highRangeAlpha = 95.0;

  Color _heatColor(double v) {
    if (v < _lowThreshold) {
      return Colors.green.withAlpha((v / _lowThreshold * _lowMaxAlpha).toInt());
    }
    if (v < _mediumThreshold) {
      return Colors.orange.withAlpha(
          ((v - _lowThreshold) / _lowThreshold * _lowMaxAlpha + _mediumBaseAlpha).toInt());
    }
    return Colors.red.withAlpha(
        ((v - _mediumThreshold) / (1.0 - _mediumThreshold) * _highRangeAlpha + _highBaseAlpha)
            .toInt());
  }

  @override
  bool shouldRepaint(_HeatmapPainter old) => old.grid != grid;
}
