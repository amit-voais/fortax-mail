#!/usr/bin/env python3
"""Compare the calendar range-query workload with two SQLite cache limits."""

from __future__ import annotations

import argparse
import os
import random
import sqlite3
import statistics
import tempfile
import time


QUERY = """
SELECT id, account_id, ical_uid, summary, starts_at, ends_at,
       description, ical_raw, deleted
FROM calendar_events
WHERE deleted = 0 AND starts_at < ?2 AND COALESCE(ends_at, starts_at) >= ?1
ORDER BY starts_at ASC LIMIT 500
"""


def measure(path: str, cache_kib: int, starts: list[int]) -> float:
    connection = sqlite3.connect(path)
    connection.execute(f"PRAGMA cache_size=-{cache_kib}")
    connection.execute("PRAGMA mmap_size=16777216")
    started = time.perf_counter()
    for start in starts:
        end = start + 31 * 24 * 3_600_000
        connection.execute(QUERY, (start, end)).fetchall()
    elapsed = time.perf_counter() - started
    connection.close()
    return elapsed


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--events", type=int, default=20_000)
    parser.add_argument("--queries", type=int, default=120)
    parser.add_argument("--rounds", type=int, default=7)
    args = parser.parse_args()

    descriptor, path = tempfile.mkstemp(prefix="fortax-calendar-cache-", suffix=".db")
    os.close(descriptor)
    try:
        connection = sqlite3.connect(path)
        connection.executescript(
            """
            PRAGMA journal_mode=WAL;
            CREATE TABLE calendar_events (
              id INTEGER PRIMARY KEY,
              account_id INTEGER NOT NULL,
              ical_uid TEXT NOT NULL,
              summary TEXT,
              starts_at INTEGER NOT NULL,
              ends_at INTEGER,
              description TEXT,
              ical_raw TEXT,
              deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_calendar_starts ON calendar_events(starts_at);
            """
        )
        payload = "x" * 320
        connection.executemany(
            """INSERT INTO calendar_events
               (id, account_id, ical_uid, summary, starts_at, ends_at,
                description, ical_raw)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                (
                    index,
                    1,
                    f"uid-{index}",
                    f"Event {index}",
                    index * 3_600_000,
                    index * 3_600_000 + 1_800_000,
                    payload,
                    payload,
                )
                for index in range(args.events)
            ),
        )
        connection.commit()
        connection.close()

        randomizer = random.Random(8128)
        lower = max(0, args.events - 6_000)
        upper = max(lower + 1, args.events - 1_000)
        starts = [
            randomizer.randrange(lower, upper) * 3_600_000 for _ in range(args.queries)
        ]
        samples = {2_048: [], 8_192: []}
        for round_number in range(args.rounds):
            order = (2_048, 8_192) if round_number % 2 == 0 else (8_192, 2_048)
            for cache_kib in order:
                samples[cache_kib].append(measure(path, cache_kib, starts))

        medians = {key: statistics.median(value) for key, value in samples.items()}
        print(f"database_bytes={os.path.getsize(path)}")
        for cache_kib, values in samples.items():
            rounded = ", ".join(f"{value:.4f}" for value in values)
            print(
                f"cache_kib={cache_kib} median_seconds={medians[cache_kib]:.6f} "
                f"samples=[{rounded}]"
            )
        print(f"2MiB_over_8MiB_latency_ratio={medians[2_048] / medians[8_192]:.4f}")
    finally:
        for suffix in ("", "-wal", "-shm"):
            try:
                os.unlink(path + suffix)
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    main()
