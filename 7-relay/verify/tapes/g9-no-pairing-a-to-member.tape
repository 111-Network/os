# 7-relay/verify/tapes/g9-no-pairing-a-to-member.tape: the frames a hub with no pairing sends a member (G9 item 1, I13).
# One line per frame, "type length" (type 1 sealed packet, 2 announce; keepalives left out), in wire order, from the
# member's reconnect to the hub's answer to its CUSTODY_PUT for a contact the hub does not hold. Recorded by forward.sh
# (G9_WRITE_TAPE=1) and equal for a Layer 6 build (item 6) and this tree.
2 141
1 89
1 106
