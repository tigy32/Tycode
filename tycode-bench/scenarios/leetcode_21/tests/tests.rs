use leetcode_21::merge_two_lists;
use leetcode_21::node::ListNode;

// Helper to build linked list from vector
fn from_vec(v: Vec<i32>) -> Option<Box<ListNode>> {
    if v.is_empty() {
        return None;
    }
    let mut head = Some(Box::new(ListNode::new(v[0])));
    let mut current = head.as_mut().unwrap();
    for &val in &v[1..] {
        current.next = Some(Box::new(ListNode::new(val)));
        current = current.next.as_mut().unwrap();
    }
    head
}

// Helper to convert linked list to vector
fn to_vec(head: &Option<Box<ListNode>>) -> Vec<i32> {
    let mut result = vec![];
    let mut current = head.as_ref();
    while let Some(node) = current {
        result.push(node.val);
        current = node.next.as_ref();
    }
    result
}

#[test]
fn test_both_empty() {
    assert_eq!(merge_two_lists(None, None), None);
}

#[test]
fn test_first_empty() {
    let list2 = from_vec(vec![1, 3, 4]);
    let result = merge_two_lists(None, list2.clone());
    assert_eq!(to_vec(&result), vec![1, 3, 4]);
}

#[test]
fn test_second_empty() {
    let list1 = from_vec(vec![1, 2, 3]);
    let result = merge_two_lists(list1.clone(), None);
    assert_eq!(to_vec(&result), vec![1, 2, 3]);
}

#[test]
fn test_example1() {
    let list1 = from_vec(vec![1, 2, 4]);
    let list2 = from_vec(vec![1, 3, 4]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 1, 2, 3, 4, 4]);
}

#[test]
fn test_example2() {
    let list1 = from_vec(vec![]);
    let list2 = from_vec(vec![]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(result, None);
}

#[test]
fn test_example3() {
    let list1 = from_vec(vec![]);
    let list2 = from_vec(vec![0]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![0]);
}

#[test]
fn test_single_elements() {
    let list1 = from_vec(vec![1]);
    let list2 = from_vec(vec![2]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 2]);
}

#[test]
fn test_reverse_single() {
    let list1 = from_vec(vec![2]);
    let list2 = from_vec(vec![1]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 2]);
}

#[test]
fn test_duplicates() {
    let list1 = from_vec(vec![1, 1, 2]);
    let list2 = from_vec(vec![1, 3, 4]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 1, 1, 2, 3, 4]);
}

#[test]
fn test_longer_first() {
    let list1 = from_vec(vec![1, 3, 5, 7, 9]);
    let list2 = from_vec(vec![2, 4]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 2, 3, 4, 5, 7, 9]);
}

#[test]
fn test_longer_second() {
    let list1 = from_vec(vec![1, 3]);
    let list2 = from_vec(vec![2, 4, 6, 8, 10]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 2, 3, 4, 6, 8, 10]);
}

#[test]
fn test_minimum_values() {
    let list1 = from_vec(vec![-100]);
    let list2 = from_vec(vec![-100]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![-100, -100]);
}

#[test]
fn test_maximum_values() {
    let list1 = from_vec(vec![100]);
    let list2 = from_vec(vec![100]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![100, 100]);
}

#[test]
fn test_alternating() {
    let list1 = from_vec(vec![1, 3, 5]);
    let list2 = from_vec(vec![2, 4, 6]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn test_one_node_both() {
    let list1 = from_vec(vec![-50]);
    let list2 = from_vec(vec![75]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![-50, 75]);
}

#[test]
fn test_large_equal_values() {
    // Test with many equal values (50 nodes each, total 100)
    let list1 = from_vec(vec![0; 50]);
    let list2 = from_vec(vec![0; 50]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![0; 100]);
}

#[test]
fn test_random_case1() {
    let list1 = from_vec(vec![-10, 3, 7]);
    let list2 = from_vec(vec![-5, 2, 8]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![-10, -5, 2, 3, 7, 8]);
}

#[test]
fn test_random_case2() {
    let list1 = from_vec(vec![0, 5, 10]);
    let list2 = from_vec(vec![1, 6]);
    let result = merge_two_lists(list1, list2);
    assert_eq!(to_vec(&result), vec![0, 1, 5, 6, 10]);
}
