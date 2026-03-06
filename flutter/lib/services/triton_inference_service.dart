import 'dart:math';
import '../models/threat_event.dart';

/// Triton ternary model inference service.
/// Runs locally on-device via Rust FFI (Phase 2).
/// Phase 1: pure-Dart mock implementation for UI development.
class TritonInferenceService {
  static final TritonInferenceService _instance = TritonInferenceService._internal();
  factory TritonInferenceService() => _instance;
  TritonInferenceService._internal();

  final _random = Random();

  bool _modelLoaded = false;
  double _lastLatencyMs = 0;

  /// Load the ternary model from assets (stub: always succeeds in Phase 1).
  Future<void> loadModel(String modelPath) async {
    await Future.delayed(const Duration(milliseconds: 250));
    _modelLoaded = true;
  }

  /// Classify packet features using 2-bit ternary inference.
  /// Returns a [ThreatVerdict] with severity, confidence, and latency.
  Future<ThreatVerdict> inferThreat(List<double> packetFeatures) async {
    assert(_modelLoaded, 'Call loadModel() before inferring');
    final start = DateTime.now();
    // Simulate ternary quantized inference latency (< 100 ms target)
    await Future.delayed(Duration(milliseconds: 8 + _random.nextInt(40)));
    _lastLatencyMs = DateTime.now().difference(start).inMicroseconds / 1000.0;

    final score = _mockInfer(packetFeatures);
    return ThreatVerdict(
      severity: score > 0.75
          ? ThreatSeverity.critical
          : score > 0.45
              ? ThreatSeverity.medium
              : ThreatSeverity.anomaly,
      confidence: score,
      latencyMs: _lastLatencyMs,
      modelVersion: 'tern-v1.0-mock',
    );
  }

  double get lastLatencyMs => _lastLatencyMs;
  bool get isLoaded => _modelLoaded;

  // Simple mock: sum-of-features heuristic simulating ternary {-1, 0, +1} weights
  double _mockInfer(List<double> features) {
    if (features.isEmpty) return 0;
    // Ternary weights: quantize each to {-1, 0, 1}
    double acc = 0;
    for (final f in features) {
      final w = f > 0.66 ? 1.0 : f > 0.33 ? 0.0 : -1.0;
      acc += w * f;
    }
    return ((acc / features.length) + 1) / 2; // normalise to [0,1]
  }
}

class ThreatVerdict {
  final ThreatSeverity severity;
  final double confidence;
  final double latencyMs;
  final String modelVersion;

  const ThreatVerdict({
    required this.severity,
    required this.confidence,
    required this.latencyMs,
    required this.modelVersion,
  });
}
