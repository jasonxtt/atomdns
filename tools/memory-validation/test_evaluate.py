"""Reject false PASS from partial runs, bad DNS, tool failures or timing changes."""
import copy
import unittest
from evaluate import evaluate, event_failed, valid_dns_outcomes


class Gates(unittest.TestCase):
    def setUp(self):
        self.identity=dict(binaries=dict(base='old',candidate='new'),warmup=60,duration=300,excluded_from_acceptance=False,top_source='default',audit_capacity=100000,prefill=100000)
        self.rows=[];self.cleanup={}
        for pair in [1,2,3]:
            for role in ['base','candidate']:
                self.rows.append(dict(pair=pair,role=role,cpu_us_per_correct=100,rss_median_kib=1000 if role=='base' else 900,
                                      prefill_dns=dict(planned=110000,sent=110000,healthy=110000,statuses=dict(correct=110000)),audit_status=dict(total_queries=100000),before=dict(hwm_kib=2000),after=dict(hwm_kib=2000),event_failures=[],event_count=1,expected_event_count=1,
                                      dns=dict(planned=150000,sent=150000,healthy=150000,statuses=dict(correct=150000),correct_latency_ms=dict(p95=5,p99=10))))
                self.cleanup[f'r{pair}-{role}']=dict(all_owned_processes_exited=True,namespace_deleted=True,original_service_unchanged=True,existing_services_unchanged=True)
    def test_explicit_corpus_is_labeled_supplementary(self):
        self.identity['top_source']='explicit'
        result=evaluate(self.identity,self.rows,self.cleanup)
        self.assertTrue(result.get('supplementary',False))
        self.assertEqual(result['top_source'],'explicit')

    def test_bad_prefill_cannot_pass_even_when_measured_dns_is_correct(self):
        for issue in ['wrong', 'late', 'unsent', 'missing']:
            rows = copy.deepcopy(self.rows)
            dns = rows[1]['prefill_dns']
            if issue == 'wrong':
                dns['statuses'] = dict(correct=109999, wrong=1)
            elif issue == 'late':
                dns['healthy'] -= 1
            elif issue == 'unsent':
                dns['sent'] -= 1
            else:
                rows[1].pop('prefill_dns')
            self.assertEqual(evaluate(self.identity, rows, self.cleanup)['status'], 'incomplete', issue)

    def test_empty_dns_is_allowed_only_for_explicit_zero_prefill(self):
        dns = dict(planned=0, sent=0, healthy=0, statuses={})
        self.assertFalse(valid_dns_outcomes(dns))
        self.assertTrue(valid_dns_outcomes(dns, allow_empty=True))
        dns['statuses'] = dict(wrong=1)
        self.assertFalse(valid_dns_outcomes(dns, allow_empty=True))

    def test_missing_corpus_provenance_cannot_pass(self):
        self.identity.pop('top_source')
        self.assertEqual(evaluate(self.identity, self.rows, self.cleanup)['status'], 'incomplete')

    def test_complete_pass_is_only_a_scenario_pass(self):
        result=evaluate(self.identity,self.rows,self.cleanup)
        self.assertEqual(result['status'],'pass')
        self.assertFalse(result['whole_project_accepted'])
    def test_missing_pair_or_cleanup_cannot_pass(self):
        self.assertEqual(evaluate(self.identity,self.rows[:-1],self.cleanup)['status'],'incomplete')
        self.cleanup.pop('r3-candidate')
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'incomplete')
    def test_wrong_dns_skipped_sends_smoke_and_bad_events_are_not_waived(self):
        for mutation in ['wrong','unsent','event','smoke']:
            rows=copy.deepcopy(self.rows);identity=copy.deepcopy(self.identity)
            if mutation=='wrong':rows[1]['dns']['statuses']=dict(correct=149999,wrong=1)
            if mutation=='unsent':rows[1]['dns']['sent']-=1
            if mutation=='event':rows[1]['event_failures']=['failure']
            if mutation=='smoke':identity['excluded_from_acceptance']=True
            self.assertEqual(evaluate(identity,rows,self.cleanup)['status'],'incomplete',mutation)
    def test_memory_saving_cannot_hide_cpu_or_tail_regression(self):
        for key in ['cpu','p99']:
            rows=copy.deepcopy(self.rows)
            for row in rows:
                if row['role']=='candidate':
                    if key=='cpu':row['cpu_us_per_correct']=106
                    else:row['dns']['correct_latency_ms']['p99']=10.6
            self.assertEqual(evaluate(self.identity,rows,self.cleanup)['status'],'fail')
    def test_peak_uses_all_run_maxima_not_medians(self):
        self.rows[-1]['after']['hwm_kib']=2001
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'fail')
    def test_late_correct_answer_is_not_hidden_by_success_status(self):
        self.rows[-1]['dns']['healthy']-=1
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'incomplete')
    def test_missing_scheduled_event_cannot_pass(self):
        self.rows[-1]['event_count']=0
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'incomplete')
    def test_snapshot_failure_is_not_hidden_as_a_successful_measurement(self):
        self.assertTrue(event_failed(dict(action='snapshot',result='PermissionError')))
        self.assertTrue(event_failed(dict(action='snapshot',result={})))
        self.assertFalse(event_failed(dict(action='snapshot',result=dict(rss_kib=10,hwm_kib=20))))
        self.assertFalse(event_failed(dict(action='http',result=dict(status=200))))
        self.assertTrue(event_failed(dict(action='http',result=dict(status=500))))
    def test_stale_identical_binary_artifact_cannot_pass(self):
        self.identity['binaries']['candidate']=self.identity['binaries']['base']
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'incomplete')
    def test_incomplete_ring_cannot_pass(self):
        self.rows[-1]['audit_status']['total_queries']=99999
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'incomplete')
    def test_steady_rss_must_decline(self):
        for row in self.rows:row['rss_median_kib']=1000
        self.assertEqual(evaluate(self.identity,self.rows,self.cleanup)['status'],'fail')


if __name__=='__main__':unittest.main()
