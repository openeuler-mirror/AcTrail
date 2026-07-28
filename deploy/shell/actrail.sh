# AcTrail interactive shell integration.
#
# Set ACTRAIL_NGA_AUTO_LAUNCH=0 before this file is sourced to keep the
# system-provided nga command outside AcTrail.

case $- in
    *i*)
        case ${ACTRAIL_NGA_AUTO_LAUNCH:-1} in
            0 | false | no | off) ;;
            *) alias nga='actrailctl launch -- nga' ;;
        esac
        ;;
esac
