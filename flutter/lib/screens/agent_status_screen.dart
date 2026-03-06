import 'dart:async';
import 'package:flutter/material.dart';
import '../models/learning_metrics.dart';
import '../services/agent_stream_service.dart';
import '../widgets/agent_status_card.dart';

class AgentStatusScreen extends StatefulWidget {
  const AgentStatusScreen({super.key});

  @override
  State<AgentStatusScreen> createState() => _AgentStatusScreenState();
}

class _AgentStatusScreenState extends State<AgentStatusScreen> {
  LearningMetrics? _metrics;
  late StreamSubscription<LearningMetrics> _sub;

  @override
  void initState() {
    super.initState();
    _sub = AgentStreamService().learningStream().listen((m) {
      if (!mounted) return;
      setState(() => _metrics = m);
    });
  }

  @override
  void dispose() {
    _sub.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('🤖 Agent Status',
              style: TextStyle(
                  fontSize: 16,
                  fontWeight: FontWeight.bold,
                  color: Colors.deepOrangeAccent)),
          const SizedBox(height: 12),
          AgentStatusCard(
            agentName: 'Threat Hunter',
            icon: '🎯',
            description: 'Zero-day detections · Suspicious DNS · C2 traffic',
            stream: AgentStreamService().threatHunterStream(),
            color: Colors.red,
          ),
          const SizedBox(height: 8),
          AgentStatusCard(
            agentName: 'Sandbox',
            icon: '🧪',
            description: 'File detonation · Malware verdicts',
            stream: AgentStreamService().sandboxStream(),
            color: Colors.orange,
          ),
          const SizedBox(height: 8),
          AgentStatusCard(
            agentName: 'IoT Firewall',
            icon: '🔌',
            description: 'Device connections · Camera blocks · Rogue AP',
            stream: AgentStreamService().iotFirewallStream(),
            color: Colors.cyanAccent,
          ),
          const SizedBox(height: 8),
          AgentStatusCard(
            agentName: 'AI Guardian',
            icon: '🛡️',
            description: 'LLM monitoring · PII protection · Data exfiltration',
            stream: AgentStreamService().aiGuardianStream(),
            color: Colors.purpleAccent,
          ),
          const SizedBox(height: 8),
          AgentStatusCard(
            agentName: 'Learning & Adaptation',
            icon: '📚',
            description: 'Model retraining · Pattern updates · RAG intel',
            stream: AgentStreamService().learningStream(),
            color: Colors.greenAccent,
          ),
          if (_metrics != null) ...[
            const SizedBox(height: 16),
            const Text('📈 Latest Learning Metrics',
                style: TextStyle(
                    fontSize: 14,
                    fontWeight: FontWeight.bold,
                    color: Colors.deepOrangeAccent)),
            const SizedBox(height: 8),
            _metricsPanel(_metrics!),
          ],
        ],
      ),
    );
  }

  Widget _metricsPanel(LearningMetrics m) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
          color: Colors.grey[900], borderRadius: BorderRadius.circular(10)),
      child: Column(children: [
        _row('Model Accuracy', '${(m.modelAccuracy * 100).toStringAsFixed(2)}%'),
        _row('Retrain Cycles', '${m.retrainCount}'),
        _row('New Patterns', '${m.newPatternsLearned}'),
        _row('Intel Updates', '${m.threatIntelUpdates}'),
        _row('Avg Inference', '${m.inferenceLatencyMs.toStringAsFixed(1)} ms'),
      ]),
    );
  }

  Widget _row(String label, String value) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 3),
        child: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(label, style: const TextStyle(color: Colors.white54, fontSize: 12)),
            Text(value,
                style: const TextStyle(
                    color: Colors.white, fontSize: 12, fontWeight: FontWeight.w600)),
          ],
        ),
      );
}
