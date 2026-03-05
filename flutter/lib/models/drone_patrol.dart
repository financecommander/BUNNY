class DronePatrol {
  final String id;
  final String label;
  final double latitude;
  final double longitude;
  final double altitudeKm;
  final double coverageRadiusDeg;
  final bool active;
  final List<PatrolWaypoint> path;

  const DronePatrol({
    required this.id,
    required this.label,
    required this.latitude,
    required this.longitude,
    required this.altitudeKm,
    required this.coverageRadiusDeg,
    required this.active,
    required this.path,
  });
}

class PatrolWaypoint {
  final double latitude;
  final double longitude;

  const PatrolWaypoint({required this.latitude, required this.longitude});
}
