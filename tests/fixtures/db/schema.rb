ActiveRecord::Schema.define do
  create_table "users", force: :cascade do |t|
    t.string "name"
    t.integer "age", null: false
    t.boolean "admin"
    t.jsonb "settings"
  end
end
