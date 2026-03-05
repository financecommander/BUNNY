enum ThreatSeverity { critical, medium, anomaly }

class ThreatEvent {
  final String id;
  final DateTime timestamp;
  final String agentName;
  final ThreatSeverity severity;
  final String description;
  final double latitude;
  final double longitude;
  final double confidence;
  final String? targetIp;

  const ThreatEvent({
    required this.id,
    required this.timestamp,
    required this.agentName,
    required this.severity,
    required this.description,
    required this.latitude,
    required this.longitude,
    required this.confidence,
    this.targetIp,
  });

  String get severityLabel {
    switch (severity) {
      case ThreatSeverity.critical:
        return 'CRITICAL';
      case ThreatSeverity.medium:
        return 'MEDIUM';
      case ThreatSeverity.anomaly:
        return 'ANOMALY';
    }
  }
}
