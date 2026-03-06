enum DeviceType { iot, mobile, desktop, unknown }
enum LicenseStatus { free, paid, unlicensed }

class IotDevice {
  final String id;
  final String ip;
  final String label;
  final DeviceType type;
  final LicenseStatus license;
  final bool protectionEnabled;
  final double latitude;
  final double longitude;
  final DateTime lastSeen;
  final int blockedAttempts;

  const IotDevice({
    required this.id,
    required this.ip,
    required this.label,
    required this.type,
    required this.license,
    required this.protectionEnabled,
    required this.latitude,
    required this.longitude,
    required this.lastSeen,
    required this.blockedAttempts,
  });

  IotDevice copyWith({bool? protectionEnabled, LicenseStatus? license}) {
    return IotDevice(
      id: id,
      ip: ip,
      label: label,
      type: type,
      license: license ?? this.license,
      protectionEnabled: protectionEnabled ?? this.protectionEnabled,
      latitude: latitude,
      longitude: longitude,
      lastSeen: lastSeen,
      blockedAttempts: blockedAttempts,
    );
  }
}
