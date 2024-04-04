use serde_json::{value::Value, Map};

pub(super) fn update_object_array_in_place<F>(input: Option<&mut Value>, mut mapper: F)
where
    F: FnMut(&mut Map<String, Value>),
{
    if let Some(Value::Array(objects)) = input {
        for object in objects.iter_mut() {
            if let Value::Object(object) = object {
                mapper(object)
            }
        }
    }
}

pub(super) fn update_object_array_skipless<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mut mapper: F,
) where
    F: FnMut(&mut Map<String, Value>),
{
    update_item(map, input_key, output_key, |item| {
        if let Value::Array(objects) = item {
            let mut objects = objects.clone();

            for object in objects.iter_mut() {
                if let Value::Object(object) = object {
                    mapper(object);
                }
            }

            Some(Value::Array(objects))
        } else {
            None
        }
    })
}

pub(super) fn update_object_array<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mut mapper: F,
) where
    F: FnMut(&Map<String, Value>) -> Option<Map<String, Value>>,
{
    update_item(map, input_key, output_key, |item| {
        if let Value::Array(objects) = item {
            let mut new_objects = Vec::with_capacity(objects.len());

            for object in objects.iter() {
                if let Value::Object(object) = object {
                    if let Some(output) = mapper(object) {
                        new_objects.push(Value::Object(output));
                    }
                }
            }

            if new_objects.is_empty() {
                None
            } else {
                Some(Value::Array(new_objects))
            }
        } else {
            None
        }
    })
}

pub(super) fn update_array<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mut mapper: F,
) where
    F: FnMut(&Value) -> Option<Value>,
{
    update_item(map, input_key, output_key, |item| {
        if let Value::Array(objects) = item {
            let mut new_objects = Vec::with_capacity(objects.len());

            for object in objects.iter() {
                if let Some(output) = mapper(object) {
                    new_objects.push(output);
                }
            }

            if new_objects.is_empty() {
                None
            } else {
                Some(Value::Array(new_objects))
            }
        } else {
            None
        }
    })
}

pub(super) fn update_array_single<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mut mapper: F,
) where
    F: FnMut(&Value) -> Option<Value>,
{
    update_item(map, input_key, output_key, |item| {
        if let Value::Array(objects) = item {
            for object in objects {
                if let Some(value) = mapper(object) {
                    return Some(value);
                }
            }

            None
        } else {
            None
        }
    })
}

pub(super) fn update_array_skipless<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mut mapper: F,
) where
    F: FnMut(&mut Value),
{
    update_item(map, input_key, output_key, |item| {
        if let Value::Array(objects) = item {
            let mut objects = objects.clone();

            for object in objects.iter_mut() {
                mapper(object);
            }

            Some(Value::Array(objects))
        } else {
            None
        }
    })
}

pub(super) fn update_item<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mapper: F,
) where
    F: FnOnce(&Value) -> Option<Value>,
{
    if !map.contains_key(output_key) {
        if let Some(value) = map.get(input_key) {
            if let Some(output) = mapper(value) {
                map.remove(input_key);
                map.insert(output_key.to_string(), output);
            }
        }
    }
}

pub(super) fn update_item_skipless<F>(
    map: &mut Map<String, Value>,
    input_key: &str,
    output_key: &str,
    mapper: F,
) where
    F: FnOnce(&mut Value),
{
    if !map.contains_key(output_key) {
        if let Some(value) = map.get_mut(input_key) {
            mapper(value);

            let value = value.clone();

            map.remove(input_key);
            map.insert(output_key.to_string(), value);
        }
    }
}

pub(super) fn update_item_in_place<F>(input: Option<&mut Value>, mapper: F)
where
    F: FnOnce(&mut Value),
{
    if let Some(value) = input {
        mapper(value)
    }
}

pub(super) fn insert_item(map: &mut Map<String, Value>, key: &str, value: Value) {
    if !map.contains_key(key) {
        map.insert(key.to_string(), value);
    }
}

pub(super) fn pop_map_item<F, V>(map: &mut Map<String, Value>, key: &str, mapper: F) -> Option<V>
where
    F: FnOnce(&Value) -> Option<V>,
{
    if let Some(value) = map.get(key) {
        if let Some(result) = mapper(value) {
            map.remove(key);

            Some(result)
        } else {
            None
        }
    } else {
        None
    }
}
