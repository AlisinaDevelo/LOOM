# Database isolation notes

Serializable isolation prevents retry anomalies from becoming silent state corruption. The
application still needs an explicit retry budget and idempotent effects.
