import unittest

from timing_attribution import attribute


class AttributionTests(unittest.TestCase):
    def test_components_preserve_schedule_and_failures(self):
        rows = [dict(seq=1, planned=100, sent=100.002, finished=100.005, status='observed_noerror'),
                dict(seq=2, planned=101, sent=None, finished=None, status='unsent'),
                dict(seq=3, planned=102, sent=102.001, finished=None, status='timeout'),
                dict(seq=4, planned=103, sent=103.001, finished=103.003, status='observed_servfail'),
                dict(seq=5, planned=104, sent=104.001, finished=104.002, status='observed_noerror')]
        result = attribute(rows, 100, 4)
        self.assertEqual((result['planned'], result['sent'], result['replies']), (4, 3, 2))
        self.assertEqual(result['unsent_sequences'], [2])
        self.assertEqual(result['statuses']['timeout'], 1)
        self.assertEqual(result['statuses']['observed_servfail'], 1)
        self.assertAlmostEqual(result['dispatch_ms']['max'], 2)
        self.assertAlmostEqual(result['socket_roundtrip_ms']['max'], 3)
        self.assertAlmostEqual(result['planned_to_receive_ms']['max'], 5)
        self.assertFalse(result['timing_accepted'])

    def test_empty_and_invalid_observations(self):
        self.assertIsNone(attribute([], 0, 1)['dispatch_ms']['p99'])
        valid = dict(seq=1, planned=100, sent=100.001, finished=100.002, status='observed_noerror')
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            attribute([valid, valid], 100, 1)
        for changes in [dict(sent=None), dict(sent=99), dict(finished=100)]:
            with self.assertRaisesRegex(ValueError, 'timestamps'):
                attribute([{**valid, **changes}], 100, 1)

    def test_component_percentiles_are_not_additive(self):
        rows = [dict(seq=1, planned=100, sent=100.009, finished=100.010, status='observed_noerror'),
                dict(seq=2, planned=101, sent=101.001, finished=101.010, status='observed_noerror')]
        result = attribute(rows, 100, 2)
        component_sum = result['dispatch_ms']['p95'] + result['socket_roundtrip_ms']['p95']
        self.assertAlmostEqual(component_sum, 18)
        self.assertAlmostEqual(result['planned_to_receive_ms']['p95'], 10)


if __name__ == '__main__':
    unittest.main()
