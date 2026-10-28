"""Paired observations retain every run and cannot become project acceptance."""
import copy
import unittest
from real_pair_summary import summarize_pairs


class RealPairSummary(unittest.TestCase):
    def reports(self):
        reports=[]
        for i in range(3):
            metric=dict(planned=150000,raw_rows=180000,protocol_replies=150000,sent=150000,
                        statuses=dict(observed_noerror=150000),sequence_sha256=str(i),
                        rss_median_kib=1000,cpu_us_per_protocol_reply=10,reply_latency_ms=dict(p95=1,p99=2))
            roles={r:dict(timed=copy.deepcopy(metric),worker_exit=0,fill_exit=0,load_exit=0,
                    fill_summary=dict(planned=100000,received=100000),pre_load_stats=dict(total_queries=100000),
                    audit_stats=dict(total_queries=100000),ready=dict(hwm_kib=3000),after_load=dict(hwm_kib=2000),
                    after_idle=dict(rss_kib=900,hwm_kib=2000),
                    load_events=[dict(action='snapshot',result=dict(rss_kib=1000,hwm_kib=3000))]) for r in ['base','candidate']}
            roles['candidate']['timed']['rss_median_kib']=800
            reports.append(dict(role_order=['candidate','base'] if i==1 else ['base','candidate'],
                        timed_spec=dict(warmup=60,duration=300,rate=500,seed=501+i,idle=30),
                        binaries=dict(base='old',candidate='new'),weighted_corpus_sha256='corpus',effective_upstream_sha256='upstream',tools_sha256={'code':'same'},roles=roles,
                        cleanup=dict(all_owned_parents_exited=True,existing_service_identity_unchanged=True,owned_live_group_members=[])))
        return reports

    def test_incomplete_pair_is_not_an_observation_complete_result(self):
        self.assertEqual(summarize_pairs(self.reports()[:2])['status'],'incomplete')

    def test_medians_and_all_run_maximum_are_separate_and_not_acceptance(self):
        reports=self.reports()
        reports[2]['roles']['candidate']['ready']['hwm_kib']=4000
        result=summarize_pairs(reports)
        self.assertEqual(result['status'],'observations_complete')
        self.assertEqual(result['metrics']['rss_median_kib']['candidate_median'],800)
        self.assertEqual(result['peak_hwm_kib']['candidate'],4000)
        self.assertFalse(result['whole_project_accepted'])
        self.assertEqual(len(result['metrics']['rss_median_kib']['candidate_runs']),3)

    def test_input_or_cleanup_change_is_not_waived(self):
        for issue in ['input','cleanup','missing_stage']:
            reports=self.reports()
            if issue=='input':reports[1]['roles']['candidate']['timed']['sequence_sha256']='bad'
            elif issue=='cleanup':reports[1]['cleanup']['owned_live_group_members']=[123]
            else:reports[1]['roles']['base'].pop('timed')
            self.assertEqual(summarize_pairs(reports)['status'],'incomplete',issue)

    def test_failed_packets_stay_visible_even_when_collection_is_complete(self):
        reports=self.reports()
        value=reports[1]['roles']['candidate']['timed']
        value['statuses']=dict(observed_noerror=149999,timeout=1)
        value['protocol_replies']=149999
        result=summarize_pairs(reports)
        self.assertEqual(result['outcomes']['candidate'][1]['statuses']['timeout'],1)
        self.assertFalse(result['correctness_checked'])
