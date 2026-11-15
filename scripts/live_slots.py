# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Live-check slots shared by all repository trees (spec AC-104), separate from cargo's pool.
TM_LIVE_SLOTS sets the worker count; the landing has a reserved slot besides those workers."""
import os
import time

from check_slots import slot as pool_slot

DEFAULT = 2


def count():
    try:
        return max(1, int(os.environ["TM_LIVE_SLOTS"]))
    except (KeyError, ValueError):
        return DEFAULT


def slot(root, step, landing=False, say=print, sleep=time.sleep):
    amount = 1 if landing else count()
    units = "slot" if amount == 1 else "slots"
    return pool_slot(root, step, "live", amount, f"a live-check slot ({amount} {units})", landing, say, sleep)
