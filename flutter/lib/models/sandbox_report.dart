class SandboxReport {
  final String id;
  final DateTime timestamp;
  final String filename;
  final String verdict;
  final bool c2Detected;
  final List<String> indicators;
  final double maliciousScore;

  const SandboxReport({
    required this.id,
    required this.timestamp,
    required this.filename,
    required this.verdict,
    required this.c2Detected,
    required this.indicators,
    required this.maliciousScore,
  });
}
