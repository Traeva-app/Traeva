"""Versioned historical evidence. Validation never restores a live object binding."""
import math
import re
import stat
from urllib.parse import urlsplit

MAX_FILE = 20 * 1024 * 1024
CHROME = "TRAEVA_CHROME_FETCH_ACQUISITION"
DIRECT = "TRAEVA_DIRECT_HTTPS_ACQUISITION"
HEADERS = {"content-length", "content-type", "content-disposition", "content-encoding"}
FIELDS = {
    "receiptVersion", "receiptKind", "sourceNamespace", "state", "acquisitionId",
    "binding", "response", "destination", "stagingName", "expectedBytes", "receivedBytes",
    "nextSequence", "startedAtNs", "observedAtNs", "createdObject", "targetCreatedObject",
    "finalObject", "streamSha256", "deliveryMethod", "quarantinePresent",
    "currentPathRequiresRecheck", "chromeDownloadId", "claimScope",
}
SCOPE = "Historical response-to-held-object observation; not product VERIFIED"
METHOD = "frozen_held_fd_snapshot_to_exclusive_target_fd"


def require(condition):
    if not condition:
        raise ValueError("invalid_historical_receipt")


def integer(value, minimum=0, maximum=(1 << 63) - 1):
    require(type(value) is int and minimum <= value <= maximum)


def text(value, maximum=512):
    require(type(value) is str and 0 < len(value.encode("utf-8")) <= maximum and "\x00" not in value)


def component(value):
    text(value, 180)
    require(not value.startswith(".") and "/" not in value and "\\" not in value)


def object_evidence(value):
    require(type(value) is dict and set(value) == {
        "device", "inode", "birthTime", "size", "links", "mode", "mtimeNs", "ctimeNs"})
    integer(value["device"])
    integer(value["inode"], 1)
    integer(value["size"], 0, MAX_FILE)
    integer(value["links"], 1, 1)
    integer(value["mode"], 0, 0xffff)
    require(stat.S_ISREG(value["mode"]))
    integer(value["mtimeNs"], 1)
    integer(value["ctimeNs"], 1)
    require(type(value["birthTime"]) in (int, float) and
            math.isfinite(value["birthTime"]) and 0 < value["birthTime"] < (1 << 63))


def identity(value):
    return value["device"], value["inode"], value["birthTime"]


def validate(record, filename):
    require(type(record) is dict and set(record) == FIELDS)
    integer(record["receiptVersion"], 1, 1)
    require(record["receiptKind"] == "HISTORICAL_ACQUISITION_OBSERVATION")
    require(record["state"] == "OBSERVATION_RECORDED")
    require(record["sourceNamespace"] in (CHROME, DIRECT))
    component(record["acquisitionId"])
    require(filename == record["acquisitionId"] + ".receipt.json")
    component(record["destination"])
    require(record["stagingName"] == record["acquisitionId"] + ".part")
    integer(record["expectedBytes"], 1, MAX_FILE)
    integer(record["receivedBytes"], 1, MAX_FILE)
    require(record["receivedBytes"] == record["expectedBytes"])
    integer(record["nextSequence"], 1, MAX_FILE)
    integer(record["startedAtNs"], 1)
    integer(record["observedAtNs"], record["startedAtNs"])
    require(type(record["streamSha256"]) is str and re.fullmatch(r"[0-9a-f]{64}", record["streamSha256"]) is not None)
    require(record["deliveryMethod"] == METHOD and record["claimScope"] == SCOPE)
    require(record["quarantinePresent"] is True and record["currentPathRequiresRecheck"] is True)
    require(record["chromeDownloadId"] is None)
    for name in ("createdObject", "targetCreatedObject", "finalObject"):
        object_evidence(record[name])
    require(record["createdObject"]["size"] == record["targetCreatedObject"]["size"] == 0)
    require(record["finalObject"]["size"] == record["expectedBytes"])
    require(identity(record["targetCreatedObject"]) == identity(record["finalObject"]))
    require(identity(record["createdObject"]) != identity(record["finalObject"]))
    binding = record["binding"]
    require(type(binding) is dict)
    if record["sourceNamespace"] == DIRECT:
        require(set(binding) == {"kind", "requestId", "responseHandle"} and binding["kind"] == "TRAEVA_DIRECT_HTTPS")
        text(binding["requestId"]); text(binding["responseHandle"])
    else:
        require(set(binding) == {"tabId", "sessionId", "debuggerEpoch", "fetchRequestId", "stream", "networkId"})
        integer(binding["tabId"])
        for name in ("debuggerEpoch", "fetchRequestId", "stream"):
            text(binding[name])
        for name in ("sessionId", "networkId"):
            require(binding[name] is None or type(binding[name]) is str and 0 < len(binding[name]) <= 512)
    response = record["response"]
    require(type(response) is dict and set(response) == {"url", "status", "headers"})
    text(response["url"], 2048)
    url = urlsplit(response["url"])
    require(url.scheme == "https" and bool(url.hostname) and not url.username and not url.password and not url.query and not url.fragment)
    integer(response["status"], 200, 200)
    require(type(response["headers"]) is list and 1 <= len(response["headers"]) <= 4)
    names = set()
    for header in response["headers"]:
        require(type(header) is dict and set(header) == {"name", "value"})
        text(header["name"]); text(header["value"], 256)
        name = header["name"].lower()
        require(name in HEADERS and name not in names)
        names.add(name)
        if name == "content-length":
            require(header["value"] == str(record["expectedBytes"]))
        elif name == "content-encoding":
            require(header["value"].lower() == "identity")
    require("content-length" in names)
    return record


def make(task):
    record = {key: task[key] for key in FIELDS - {"receiptVersion", "receiptKind"}}
    record.update(receiptVersion=1, receiptKind="HISTORICAL_ACQUISITION_OBSERVATION")
    return validate(record, record["acquisitionId"] + ".receipt.json")


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value)
        value[key] = item
    return value
