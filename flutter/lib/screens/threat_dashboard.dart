import 'dart:async';
import 'package:flutter/material.dart';
import '../models/threat_event.dart';
import '../models/learning_metrics.dart';
import '../services/agent_stream_service.dart';
import '../services/triton_inference_service.dart';
import '../widgets/anomaly_feed.dart';
import '../widgets/threat_heatmap.dart';

/// Real-time threat dashboard screen.
class ThreatDashboard extends StatefulWidget {
  const ThreatDashboard({super.key});

  @override
  State<ThreatDashboard> createState() => _ThreatDashboardState();
}

class _ThreatDashboardState extends State<ThreatDashboard> {
  int _critical = 0;
  int _medium = 0;
  int _anomalies = 0;
  int _totalDetections = 0;
  double _inferenceLatency = 0;
  LearningMetrics? _lastMetrics;

  late StreamSubscription<ThreatEvent> _threatSub;
  late StreamSubscription<LearningMetrics> _learningSub;

  @override
  void initState() {
    super.initState();
    TritonInferenceService().loadModel('assets/models/threat_classifier.tern');

    _threatSub = AgentStreamService().threatHunterStream().listen((event) async {
      final verdict = await TritonInferenceService()
          .inferThreat(List.generate(16, (i) => event.confidence * (i % 3 == 0 ? 1 : 0.5)));
      if (!mounted) return;
      setState(() {
        _totalDetections++;
        _inferenceLatency = verdict.latencyMs;
        switch (event.severity) {
          case ThreatSeverity.critical:
            _critical++;
          case ThreatSeverity.medium:
            _medium++;
          case ThreatSeverity.anomaly:
            _anomalies++;
        }
      });
    });

    _learningSub = AgentStreamService().learningStream().listen((m) {
      if (!mounted) return;
      setState(() => _lastMetrics = m);
    });
  }

  @override
  void dispose() {
    _threatSub.cancel();
    _learningSub.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _sectionTitle('📊 Live Threat Metrics'),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(child: _metricCard('Critical', _critical, Colors.red)),
              const SizedBox(width: 8),
              Expanded(child: _metricCard('Medium', _medium, Colors.orange)),
              const SizedBox(width: 8),
              Expanded(child: _metricCard('Anomaly', _anomalies, Colors.greenAccent)),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                  child: _metricCard(
                      'Total Events', _totalDetections, Colors.cyanAccent)),
              const SizedBox(width: 8),
              Expanded(
                  child: _metricCard(
                      'Inference', null, Colors.deepOrangeAccent,
                      subtitle: '${_inferenceLatency.toStringAsFixed(1)} ms')),
            ],
          ),
          const SizedBox(height: 16),
          _sectionTitle('🌡️ Threat Heatmap'),
          const SizedBox(height: 8),
          const ThreatHeatmap(),
          const SizedBox(height: 16),
          if (_lastMetrics != null) ...[
            _sectionTitle('🤖 Learning & Adaptation'),
            const SizedBox(height: 8),
            _learningCard(_lastMetrics!),
            const SizedBox(height: 16),
          ],
          _sectionTitle('📜 Anomaly Feed'),
          const SizedBox(height: 8),
          const SizedBox(height: 320, child: AnomalyFeed()),
        ],
      ),
    );
  }

  Widget _sectionTitle(String title) => Text(title,
      style: const TextStyle(
          fontSize: 16, fontWeight: FontWeight.bold, color: Colors.deepOrangeAccent));

  Widget _metricCard(String label, int? value, Color color, {String? subtitle}) {
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 12, horizontal: 8),
      decoration: BoxDecoration(
        color: Colors.grey[900],
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: color.withAlpha(100)),
      ),
      child: Column(
        children: [
          Text(label, style: TextStyle(color: Colors.grey[400], fontSize: 11)),
          const SizedBox(height: 4),
          Text(subtitle ?? '${value ?? 0}',
              style: TextStyle(color: color, fontSize: 24, fontWeight: FontWeight.bold)),
        ],
      ),
    );
  }

  Widget _learningCard(LearningMetrics m) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
          color: Colors.grey[900], borderRadius: BorderRadius.circular(10)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _learningRow('Accuracy', '${(m.modelAccuracy * 100).toStringAsFixed(2)}%'),
          _learningRow('Retrain cycles', '${m.retrainCount}'),
          _learningRow('New patterns', '${m.newPatternsLearned}'),
          _learningRow('Intel updates', '${m.threatIntelUpdates}'),
          _learningRow('Avg inference', '${m.inferenceLatencyMs.toStringAsFixed(1)} ms'),
        ],
      ),
    );
  }

  Widget _learningRow(String label, String value) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(label, style: const TextStyle(color: Colors.white60, fontSize: 12)),
            Text(value,
                style: const TextStyle(
                    color: Colors.white, fontSize: 12, fontWeight: FontWeight.w600)),
          ],
        ),
      );
}
