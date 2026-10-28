"""Observations cannot silently become correctness or performance acceptance."""
import unittest
from real_metrics import observed_status, summarize


class RealMetrics(unittest.TestCase):
    def test_live_reply_never_claims_correct_and_question_must_match(self):
        dns=dict(rcode=0,questions=[('x.example.',1,1)],answers=[],authority=[],additional=[])
        self.assertEqual(observed_status(dns,'x.example',1),'observed_noerror')
        self.assertEqual(observed_status(dns,'other.example',1),'observed_wrong_question')
        dns['rcode']=2
        self.assertEqual(observed_status(dns,'x.example',1),'observed_servfail')

    def test_only_measured_requests_and_protocol_replies_enter_denominator(self):
        rows=[dict(index=0,planned=99,sent=99,finished=99.1,status='observed_noerror'),
              dict(index=1,planned=100,sent=100,finished=100.01,status='observed_noerror'),
              dict(index=2,planned=101,sent=101,finished=101.6,status='observed_servfail'),
              dict(index=3,planned=102,sent=102,finished=None,status='timeout'),
              dict(index=4,planned=103,sent=None,finished=None,status='unsent')]
        samples=[dict(time=100,processes=dict(sut=dict(cpu_s=10,rss_kib=100))),
                 dict(time=104,processes=dict(sut=dict(cpu_s=12,rss_kib=80)))]
        result=summarize(rows,samples,100,4)
        self.assertEqual(result['planned'],4)
        self.assertEqual(result['protocol_replies'],2)
        self.assertEqual(result['on_time_500ms'],1)
        self.assertEqual(result['cpu_us_per_protocol_reply'],1000000)
        self.assertFalse(result['correctness_checked'])
        self.assertFalse(result['whole_project_accepted'])
        self.assertEqual(result['statuses']['timeout'],1)
