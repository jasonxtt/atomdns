import unittest
import copy
from operation_summary import dns_outcomes, memory_gate, event_gate, parse_api_response, validate_controls


class OperationSummaryTests(unittest.TestCase):
    def test_sender_waiver_preserves_counts_and_only_exempts_unsent(self):
        rows=[dict(index=99,status='unsent'),dict(index=3972,status='observed_servfail')]
        self.assertFalse(dns_outcomes(rows)['ok'])
        allowed=dns_outcomes(rows,ignore_unsent=True)
        self.assertTrue(allowed['ok'])
        self.assertEqual(allowed['statuses']['unsent'],1)
        self.assertEqual(allowed['ignored_unsent'],1)
        self.assertEqual(allowed['raw_unexpected'],1)
        for status in ['timeout','send_error','observed_wrong_question','observed_servfail']:
            self.assertFalse(dns_outcomes([dict(index=99,status=status)],ignore_unsent=True)['ok'])

    def test_resume_controls_must_match_identity_and_all_three_pairs(self):
        identity=dict(base='b',candidate='c',input='i',corpus='q',core='d')
        prior=dict(identity=dict(identity),R2={'M0':{str(p):{r:dict(ok=True,rss145_kib=10,idle=dict(rss_kib=11)) for r in ['base','candidate']} for p in [1,2,3]}})
        self.assertTrue(validate_controls(prior,identity))
        altered=copy.deepcopy(identity);altered['corpus']='other'
        with self.assertRaises(ValueError):validate_controls(prior,altered)
        del prior['R2']['M0']['3']['candidate']
        with self.assertRaises(ValueError):validate_controls(prior,identity)

    def test_native_success_can_be_json_plain_text_or_empty(self):
        self.assertEqual(parse_api_response('POST',200,'ok'),'ok')
        self.assertIsNone(parse_api_response('DELETE',204,''))
        self.assertEqual(parse_api_response('GET',200,'{"capacity":100000}'),{'capacity':100000})
        with self.assertRaises(ValueError):parse_api_response('GET',200,'ok')
        with self.assertRaises(ValueError):parse_api_response('GET',204,'')
        with self.assertRaises(ValueError):parse_api_response('POST',500,'ok')
    def test_only_confirmed_question_servfail_is_exempt(self):
        rows=[dict(index=3972,status='observed_servfail'),dict(index=1,status='observed_noerror')]
        self.assertEqual(dns_outcomes(rows)['known_servfail'],1)
        rows.append(dict(index=3972,status='timeout'))
        self.assertFalse(dns_outcomes(rows)['ok'])
        self.assertFalse(dns_outcomes([dict(index=99,status='observed_servfail')])['ok'])

    def test_gate_uses_role_medians_all_run_cap_and_matching_control(self):
        self.assertTrue(memory_gate([100,100,200],[100,101,101],[[1,1],[1,1],[1,1]])['ok'])
        self.assertFalse(memory_gate([100]*3,[102]*3,[[1,1]]*3)['ok'])
        self.assertFalse(memory_gate([100]*3,[100,100,262145],[[1,1]]*3)['ok'])
        self.assertFalse(memory_gate([100]*3,[100]*3,[[1,1],[1,1.201],[1,1]])['ok'])
        with self.assertRaises(ValueError):memory_gate([100],[100],[[1,1]])

    def test_missing_failed_late_or_unreset_snapshots_do_not_pass(self):
        events=[dict(action='snapshot',at=35,started=35,result=dict(rss_kib=1,hwm_kib=2),reset_hwm=True),
                dict(action='snapshot',at=145,started=145,result=dict(rss_kib=1,hwm_kib=2))]
        self.assertTrue(event_gate(events,0,0))
        self.assertFalse(event_gate(events[:1],0,0))
        late=copy.deepcopy(events);late[1]['started']=147
        self.assertFalse(event_gate(late,0,0))
        failed=copy.deepcopy(events)+[dict(action='http',at=40,started=40,result=dict(status=500))]
        self.assertFalse(event_gate(failed,0,1))
        deleted=copy.deepcopy(events)+[dict(action='http',at=40,started=40,result=dict(status=204))]
        self.assertTrue(event_gate(deleted,0,1))
        events[0]['reset_hwm']=False
        self.assertFalse(event_gate(events,0,0))

if __name__=='__main__':unittest.main()
