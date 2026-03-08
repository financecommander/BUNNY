import 'dart:async';
import 'dart:math';
import '../models/threat_event.dart';
import '../models/sandbox_report.dart';
import '../models/iot_device.dart';
import '../models/ai_guardian_alert.dart';
import '../models/learning_metrics.dart';

/// Provides mock real-time streams from each BUNNY agent.
/// Replace stream bodies with actual Rust FFI / WebSocket calls in Phase 2.
class AgentStreamService {
  static final AgentStreamService _instance = AgentStreamService._internal();
  factory AgentStreamService() => _instance;
  AgentStreamService._internal();

  final _random = Random();

  // ── Threat Hunter ───────────────────────────────────────────────────────────

  Stream<ThreatEvent> threatHunterStream() async* {
    const agents = ['Threat Hunter', 'IoT Firewall', 'AI Guardian', 'Sandbox'];
    const descriptions = [
      'Suspicious DNS query to .ru domain',
      'C2 beacon detected on port 4444',
      'Port scan from unknown host',
      'Unauthorized camera access attempt',
      'LLM prompt injection detected',
      'Exfiltration attempt blocked',
      'Zero-day exploit signature matched',
      'Rogue DHCP server discovered',
    ];
    const lats = [40.7, 51.5, 35.7, -33.9, 48.9, 55.8, 19.1, 1.3];
    const lons = [-74.0, -0.1, 139.7, 151.2, 2.3, 37.6, 72.9, 103.8];

    var idx = 0;
    while (true) {
      await Future.delayed(Duration(milliseconds: 2000 + _random.nextInt(3000)));
      final sev = _random.nextInt(10);
      yield ThreatEvent(
        id: 'th-${DateTime.now().millisecondsSinceEpoch}',
        timestamp: DateTime.now(),
        agentName: agents[_random.nextInt(agents.length)],
        severity: sev < 3
            ? ThreatSeverity.critical
            : sev < 7
                ? ThreatSeverity.medium
                : ThreatSeverity.anomaly,
        description: descriptions[idx % descriptions.length],
        latitude: lats[idx % lats.length] + (_random.nextDouble() - 0.5) * 5,
        longitude: lons[idx % lons.length] + (_random.nextDouble() - 0.5) * 5,
        confidence: 0.6 + _random.nextDouble() * 0.4,
        targetIp: '192.168.${_random.nextInt(255)}.${_random.nextInt(255)}',
      );
      idx++;
    }
  }

  // ── Sandbox ─────────────────────────────────────────────────────────────────

  Stream<SandboxReport> sandboxStream() async* {
    const files = [
      'suspicious.exe',
      'invoice.pdf.js',
      'update.msi',
      'photo.jpg.bat',
    ];
    var idx = 0;
    while (true) {
      await Future.delayed(Duration(seconds: 8 + _random.nextInt(12)));
      final malicious = _random.nextBool();
      yield SandboxReport(
        id: 'sb-${DateTime.now().millisecondsSinceEpoch}',
        timestamp: DateTime.now(),
        filename: files[idx % files.length],
        verdict: malicious ? 'malicious' : 'clean',
        c2Detected: malicious && _random.nextBool(),
        indicators: malicious
            ? ['registry persistence', 'outbound beacon', 'process injection']
                .sublist(0, 1 + _random.nextInt(3))
            : [],
        maliciousScore: malicious ? 0.7 + _random.nextDouble() * 0.3 : _random.nextDouble() * 0.2,
      );
      idx++;
    }
  }

  // ── IoT Firewall ─────────────────────────────────────────────────────────────

  Stream<IotDeviceStatus> iotFirewallStream() async* {
    while (true) {
      await Future.delayed(Duration(seconds: 5 + _random.nextInt(5)));
      yield IotDeviceStatus(
        deviceId: 'dev-${_random.nextInt(10)}',
        blockedAttempts: _random.nextInt(50),
        lastActivity: DateTime.now(),
      );
    }
  }

  // ── AI Guardian ─────────────────────────────────────────────────────────────

  Stream<AiGuardianAlert> aiGuardianStream() async* {
    const actions = ['redact_and_block', 'monitor', 'allow_with_audit'];
    const reasons = [
      'PII leak detected',
      'Prompt injection attempt',
      'Sensitive data exfiltration',
      'Unusual API call pattern',
    ];
    while (true) {
      await Future.delayed(Duration(seconds: 6 + _random.nextInt(10)));
      yield AiGuardianAlert(
        id: 'ag-${DateTime.now().millisecondsSinceEpoch}',
        timestamp: DateTime.now(),
        action: actions[_random.nextInt(actions.length)],
        reason: reasons[_random.nextInt(reasons.length)],
        blocked: _random.nextBool(),
      );
    }
  }

  // ── Learning & Adaptation ────────────────────────────────────────────────────

  Stream<LearningMetrics> learningStream() async* {
    var retrainCount = 0;
    var accuracy = 0.91 + _random.nextDouble() * 0.05;
    while (true) {
      await Future.delayed(const Duration(seconds: 15));
      retrainCount++;
      accuracy = (accuracy + (_random.nextDouble() - 0.5) * 0.01).clamp(0.85, 0.999);
      yield LearningMetrics(
        timestamp: DateTime.now(),
        retrainCount: retrainCount,
        modelAccuracy: accuracy,
        newPatternsLearned: _random.nextInt(20),
        threatIntelUpdates: _random.nextInt(5),
        inferenceLatencyMs: 12 + _random.nextDouble() * 30,
      );
    }
  }
}

class IotDeviceStatus {
  final String deviceId;
  final int blockedAttempts;
  final DateTime lastActivity;

  const IotDeviceStatus({
    required this.deviceId,
    required this.blockedAttempts,
    required this.lastActivity,
  });
}
