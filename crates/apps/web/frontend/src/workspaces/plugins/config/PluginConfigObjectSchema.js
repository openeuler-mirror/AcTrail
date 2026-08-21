export default class PluginConfigObjectSchema {
  constructor(schema, value) {
    this.schema = schema;
    this.value = this.isObject(value) ? value : {};
    this.properties = this.isObject(schema.properties) ? schema.properties : {};
    this.propertyOverrides = new Map();
    this.requiredKeys = new Set(Array.isArray(schema.required) ? schema.required : []);
    this.hiddenKeys = new Set();
    this.forbiddenKeys = new Set();
    this.deferredConditionalOrder = new Map();

    const clauses = [schema, ...(Array.isArray(schema.allOf) ? schema.allOf : [])];
    for (const clause of clauses) this.applyConditional(clause);
  }

  propertyKeys() {
    return Object.keys(this.properties).filter((key) => !this.hiddenKeys.has(key));
  }

  propertySchema(key) {
    return {
      ...(this.properties[key] ?? {}),
      ...(this.propertyOverrides.get(key) ?? {}),
    };
  }

  isRequired(key) {
    return this.requiredKeys.has(key);
  }

  isDeferredConditional(key) {
    return this.deferredConditionalOrder.has(key);
  }

  conditionalOrder(key) {
    return this.deferredConditionalOrder.get(key) ?? Number.MAX_SAFE_INTEGER;
  }

  sanitize(value) {
    const sanitized = { ...(this.isObject(value) ? value : {}) };
    for (const key of this.forbiddenKeys) delete sanitized[key];
    return sanitized;
  }

  applyConditional(clause) {
    if (!this.isObject(clause)) return;
    const condition = clause.if;
    const discriminatorKeys = Array.isArray(condition?.required) ? condition.required : [];
    if (discriminatorKeys.length !== 1 || !this.isObject(clause.then) || !this.isObject(clause.else)) {
      return;
    }

    const discriminator = discriminatorKeys[0];
    const conditionSchema = condition?.properties?.[discriminator];
    if (!this.isObject(conditionSchema)
      || !Object.prototype.hasOwnProperty.call(conditionSchema, 'const')) {
      return;
    }

    const discriminatorValue = Object.prototype.hasOwnProperty.call(this.value, discriminator)
      ? this.value[discriminator]
      : this.properties[discriminator]?.default;
    const conditionMatches = Object.is(discriminatorValue, conditionSchema.const);
    const activeBranch = conditionMatches ? clause.then : clause.else;
    const inactiveBranch = conditionMatches ? clause.else : clause.then;
    this.registerDeferredConditionalOrder(clause.then, clause.else);
    this.applyActiveBranch(activeBranch);
    this.hideInactiveBranch(activeBranch, inactiveBranch);
  }

  registerDeferredConditionalOrder(thenBranch, elseBranch) {
    const thenProperties = this.isObject(thenBranch.properties) ? thenBranch.properties : {};
    const elseProperties = this.isObject(elseBranch.properties) ? elseBranch.properties : {};
    if (![...Object.values(thenProperties), ...Object.values(elseProperties)]
      .some((propertySchema) => propertySchema === false)) {
      return;
    }

    const orderedKeys = [
      ...(Array.isArray(thenBranch.required) ? thenBranch.required : []),
      ...Object.keys(thenProperties),
      ...(Array.isArray(elseBranch.required) ? elseBranch.required : []),
      ...Object.keys(elseProperties),
    ];
    for (const key of orderedKeys) {
      if (!this.deferredConditionalOrder.has(key)) {
        this.deferredConditionalOrder.set(key, this.deferredConditionalOrder.size);
      }
    }
  }

  applyActiveBranch(branch) {
    const properties = this.isObject(branch.properties) ? branch.properties : {};
    for (const [key, propertySchema] of Object.entries(properties)) {
      if (propertySchema === false) {
        this.hiddenKeys.add(key);
        this.forbiddenKeys.add(key);
      } else if (this.isObject(propertySchema)) {
        this.propertyOverrides.set(key, {
          ...(this.propertyOverrides.get(key) ?? {}),
          ...propertySchema,
        });
      }
    }
    for (const key of Array.isArray(branch.required) ? branch.required : []) {
      this.requiredKeys.add(key);
    }
  }

  hideInactiveBranch(activeBranch, inactiveBranch) {
    const activeProperties = this.isObject(activeBranch.properties) ? activeBranch.properties : {};
    const activeKeys = new Set(Array.isArray(activeBranch.required) ? activeBranch.required : []);
    for (const [key, propertySchema] of Object.entries(activeProperties)) {
      if (propertySchema !== false) activeKeys.add(key);
    }

    const inactiveProperties = this.isObject(inactiveBranch.properties)
      ? inactiveBranch.properties
      : {};
    const inactiveKeys = new Set([
      ...Object.keys(inactiveProperties),
      ...(Array.isArray(inactiveBranch.required) ? inactiveBranch.required : []),
    ]);
    for (const key of inactiveKeys) {
      if (!activeKeys.has(key)) this.hiddenKeys.add(key);
    }
  }

  isObject(value) {
    return value != null && typeof value === 'object' && !Array.isArray(value);
  }
}
