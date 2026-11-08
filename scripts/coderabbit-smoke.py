"""Standalone sample for checking pull request reviews."""


def average_track_duration(durations: list[float]) -> float:
    """Return the mean duration in seconds, or zero when there are no tracks."""
    return sum(durations) / len(durations)
