"""State-aware closed-loop comparison must not conceal learning differences."""
import copy
import unittest
from state_model import compare


class StateModel(unittest.TestCase):
    def setUp(self):
        self.rows = [dict(index=0, before=[], after=[dict(category='no6', name='x.example',
                         mark=9, conclusion='no6', reason='r', hits=0,
                         first_confirmed='2026-10-25T00:00:00Z', last_confirmed='2026-10-25T00:00:00Z',
                         expires_at='2026-10-26T00:00:00Z')],
                         started=1792886400, finished=1792886401, dns='answer', route={'domain_set': 'route'})]

    def test_learning_difference_is_not_a_permitted_answer_variant(self):
        candidate = copy.deepcopy(self.rows)
        candidate[0]['after'][0]['mark'] = 10
        self.assertEqual(compare(self.rows, candidate, 1)['status'], 'fail')

    def test_expiry_during_trace_is_incomplete_even_if_answers_match(self):
        self.rows[0]['after'][0]['expires_at'] = '2026-10-25T00:00:01Z'
        self.rows[0]['started'] = 1792886400
        self.rows[0]['finished'] = 1792886402
        self.assertEqual(compare(self.rows, self.rows, 1)['status'], 'incomplete')

    def test_missing_rows_and_nonsequential_indices_are_incomplete(self):
        self.assertEqual(compare(self.rows, [], 1)['status'], 'incomplete')
        self.rows[0]['index'] = 1
        self.assertEqual(compare(self.rows, self.rows, 1)['status'], 'incomplete')

    def test_only_absolute_confirmation_times_are_projected(self):
        candidate = copy.deepcopy(self.rows)
        candidate[0]['after'][0]['last_confirmed'] = '2026-10-25T00:00:01Z'
        self.assertEqual(compare(self.rows, candidate, 1)['status'], 'pass')
        candidate[0]['after'][0]['hits'] = 1
        self.assertEqual(compare(self.rows, candidate, 1)['status'], 'fail')

    def test_initial_expiry_is_checked_even_if_that_name_is_never_queried(self):
        initial = copy.deepcopy(self.rows[0]['after'])
        initial[0]['expires_at'] = '2026-10-25T00:00:01Z'
        self.assertEqual(compare(self.rows, self.rows, 1, initial, initial)['status'], 'incomplete')
