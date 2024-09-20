import logging
import os

from pyln.testing.fixtures import *  # noqa: F403
from pyln.testing.utils import sync_blockheight

LOGGER = logging.getLogger(__name__)

PLUGIN_PATH = os.path.join(os.path.dirname(__file__), "./stablechannels.py")


def test_start(node_factory, bitcoind):
    l1, l2 = node_factory.get_nodes(
        2,
        opts={"experimental-dual-fund": None},
    )
    funder = l2.rpc.funderupdate(
        policy="match",
        policy_mod=100,
        leases_only=True,
        lease_fee_base_msat=2_000,
        lease_fee_basis=10,
        channel_fee_max_base_msat=1000,
        channel_fee_max_proportional_thousandths=2,
    )
    l1.fundwallet(10_000_000)
    l2.fundwallet(10_000_000)

    cl1 = l1.rpc.fundchannel(
        l2.info["id"] + "@localhost:" + str(l2.port),
        1_000_000,
        request_amt=1_000_000,
        compact_lease=funder["compact_lease"],
    )
    bitcoind.generate_block(6)
    sync_blockheight(bitcoind, [l1, l2])

    # configs = l1.rpc.listconfigs()["configs"]
    l1.rpc.plugin_start(
        PLUGIN_PATH,
        **{
            "channel-id": cl1["channel_id"],
            "is-stable-receiver": False,
            "stable-dollar-amount": 400,
            "native-btc-amount": 500_000,
            "counterparty": l2.info["id"],
        },
    )
    l1.daemon.wait_for_log("Starting Stable Channel with these details")
    invoice = l2.rpc.invoice(1_000_000, "label1", "desc")
    l1.rpc.pay(invoice["bolt11"])

    invoice = l1.rpc.invoice(1_000_000, "label1", "desc")
    l2.rpc.pay(invoice["bolt11"])
    
    l1.rpc.call("dev-check-stable")


