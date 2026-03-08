class AiGuardianAlert {
  final String id;
  final DateTime timestamp;
  final String action;
  final String reason;
  final String? redactedContent;
  final bool blocked;

  const AiGuardianAlert({
    required this.id,
    required this.timestamp,
    required this.action,
    required this.reason,
    this.redactedContent,
    required this.blocked,
  });
}
