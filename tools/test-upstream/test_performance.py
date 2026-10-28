import copy
import unittest
from performance_summary import summarize,descriptive_metrics,incomplete_send_collection_allowed

def pair(index):
    metric=dict(planned=150000,sent=150000,protocol_replies=150000,raw_rows=180000,statuses={'observed_noerror':149999,'observed_servfail':1},rss_median_kib=100,cpu_us_per_protocol_reply=100,reply_latency_ms=dict(p95=1,p99=2),sequence_sha256='same')
    row=dict(timed=metric,pre_count=100000,post_count=100000,points=[dict(hwm_kib=150)],cleanup=dict(all_owned_processes_exited=True,namespace_deleted=True,original_service_unchanged=True),existing_identity=True,protected_ok=True)
    return dict(pair=index,order=['candidate','base'] if index==2 else ['base','candidate'],seed=500+index,identity={'base':'old','candidate':'new','input':'same','tools':'same','fixture':'same'},roles={'base':copy.deepcopy(row),'candidate':copy.deepcopy(row)})

class PerformanceSummary(unittest.TestCase):
    def test_incomplete_collection_never_qualifies_or_hides_missing_slots(self):
        rows=[pair(i) for i in [1,2,3]]
        metric=rows[0]['roles']['base']['timed']
        metric.update(sent=149999,protocol_replies=149999,orphan_packets=0,
                      statuses={'observed_noerror':149998,'observed_servfail':1,'unsent':1})
        original=copy.deepcopy(rows)
        self.assertTrue(incomplete_send_collection_allowed(metric))
        self.assertEqual(summarize(rows)['status'],'incomplete')
        self.assertEqual(descriptive_metrics(rows)['rss_median_kib']['base_median'],100)
        self.assertEqual(rows,original)
        for name,value in [('orphan_packets',1),('protocol_replies',149998),('raw_rows',179999),
                           ('statuses',{'observed_noerror':149998,'timeout':1,'unsent':1}),
                           ('statuses',{'observed_noerror':149998,'observed_wrong_question':1,'unsent':1})]:
            bad=copy.deepcopy(metric);bad[name]=value
            self.assertFalse(incomplete_send_collection_allowed(bad),name)
        with self.assertRaises(ValueError):descriptive_metrics(rows[:1])

    def test_medians_peaks_and_failures_stay_visible(self):
        rows=[pair(i) for i in [1,2,3]]
        for r,rss,peak in zip(rows,[80,70,75],[140,170,145]):
            r['roles']['candidate']['timed']['rss_median_kib']=rss;r['roles']['candidate']['points']=[dict(hwm_kib=peak)]
        result=summarize(rows)
        self.assertEqual(result['status'],'observations_complete')
        self.assertEqual(result['metrics']['rss_median_kib']['candidate_median'],75)
        self.assertEqual(result['max_hwm_kib']['candidate'],170)
        self.assertEqual(result['outcomes']['candidate'][0]['observed_servfail'],1)
        self.assertFalse(result['correctness_checked']);self.assertFalse(result['whole_project_accepted'])

    def test_missing_data_shortfall_identity_sequence_and_cleanup_reject(self):
        self.assertEqual(summarize([pair(1)])['status'],'incomplete')
        for mutation in ['cleanup','identity','sequence','shortfall','ring','protected']:
            rows=[pair(i) for i in [1,2,3]];r=rows[1];c=r['roles']['candidate']
            if mutation=='cleanup':c['cleanup']['namespace_deleted']=False
            elif mutation=='identity':r['identity']['tools']='changed'
            elif mutation=='sequence':c['timed']['sequence_sha256']='changed'
            elif mutation=='shortfall':c['timed']['sent']=149999
            elif mutation=='ring':c['post_count']=99999
            else:c['protected_ok']=False
            self.assertEqual(summarize(rows)['status'],'incomplete',mutation)
