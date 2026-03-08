class LearningMetrics {
  final DateTime timestamp;
  final int retrainCount;
  final double modelAccuracy;
  final int newPatternsLearned;
  final int threatIntelUpdates;
  final double inferenceLatencyMs;

  const LearningMetrics({
    required this.timestamp,
    required this.retrainCount,
    required this.modelAccuracy,
    required this.newPatternsLearned,
    required this.threatIntelUpdates,
    required this.inferenceLatencyMs,
  });
}
