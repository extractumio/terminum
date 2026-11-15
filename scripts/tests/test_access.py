# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-146: slow access replies keep AC-09's assertions; silence and trickles stay bounded."""
from pathlib import Path
import socket
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import e2e_access as access

REPLY = b'HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n{"reason":"wrong"}'


class Reply:
    """A socket whose scripted reads consume real timeout budgets on a virtual clock."""
    def __init__(self, chunks, connect=0):
        self.chunks, self.now, self.connect = list(chunks), 0, connect
        self.timeout, self.closed, self.sent, self.connections = None, False, [], 0

    def open(self, address, timeout):
        self.connections += 1
        self.now += self.connect
        self.timeout = timeout
        return self

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def close(self):
        self.closed = True

    def settimeout(self, seconds):
        self.timeout = seconds

    def sendall(self, data):
        self.sent.append(data)

    def recv(self, size):
        delay, data = self.chunks.pop(0)
        self.now += min(delay, self.timeout)
        if delay > self.timeout:
            raise socket.timeout("timed out")
        return data


class AccessDeadline(unittest.TestCase):
    def request(self, reply, load=60, cores=16):
        with mock.patch.object(access.os, "getloadavg", return_value=(load, 0, 0)), \
                mock.patch.object(access.os, "cpu_count", return_value=cores), \
                mock.patch.object(access.time, "monotonic", side_effect=lambda: reply.now), \
                mock.patch.object(access.socket, "create_connection", side_effect=reply.open):
            return access.raw(12345, "POST", "/auth/login", {"Host": "localhost"}, b'{"password":"wrong"}')

    def test_slow_wrong_password_is_still_asserted_once(self):
        reply = Reply([(11, REPLY), (0, b"")])
        status, body, head = self.request(reply)
        self.assertEqual((status, body), (401, {"reason": "wrong"}))
        self.assertIn("Connection: close", head)
        self.assertEqual((reply.connections, len(reply.sent)), (1, 1))
        self.assertIn(b'{"password":"wrong"}', reply.sent[0])
        self.assertTrue(reply.closed)

    def test_load_slack_is_bounded_and_unknown_core_count_is_safe(self):
        for load, cores, budget in [(3.2, 16, 10), (60, 16, 37.5), (128, 16, 40), (2, None, 20)]:
            with self.subTest(load=load, cores=cores):
                reply = Reply([(100, b"")])
                with self.assertRaises(TimeoutError):
                    self.request(reply, load, cores)
                self.assertEqual(reply.now, budget)
                self.assertTrue(reply.closed)

    def test_fragmented_response_is_read_to_its_end(self):
        reply = Reply([(11, REPLY[:12]), (11, REPLY[12:]), (0, b"")])
        self.assertEqual(self.request(reply)[:2], (401, {"reason": "wrong"}))
        self.assertTrue(reply.closed)

    def test_trickles_do_not_restart_the_deadline(self):
        reply = Reply([(9, b"H")] * 6)
        with self.assertRaises(TimeoutError):
            self.request(reply)
        self.assertEqual(reply.now, 37.5)
        self.assertTrue(reply.closed)

    def test_connection_time_is_part_of_the_same_deadline(self):
        reply = Reply([(2, REPLY)], connect=37)
        with self.assertRaises(TimeoutError):
            self.request(reply)
        self.assertEqual(reply.now, 37.5)
        self.assertTrue(reply.closed)


if __name__ == "__main__":
    unittest.main()
