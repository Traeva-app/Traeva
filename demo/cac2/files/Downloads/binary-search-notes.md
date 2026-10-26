# Binary search

- Works only on a sorted list.
- Look at the middle item; if the target is smaller, keep the left half, otherwise the right half.
- Each step halves the list, so it takes about log2(n) steps: 20 steps for a million items.

```python
def binary_search(items, target):
    low, high = 0, len(items) - 1
    while low <= high:
        mid = (low + high) // 2
        if items[mid] == target:
            return mid
        if items[mid] < target:
            low = mid + 1
        else:
            high = mid - 1
    return -1
```

Homework for computer science: trace the search for 37 in [3, 8, 15, 22, 37, 41, 56].
