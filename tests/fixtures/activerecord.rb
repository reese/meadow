# A base record class.
class ApplicationRecord; end

# A user model.
class User < ApplicationRecord
  has_many :posts, dependent: :destroy
  belongs_to :account, class_name: "Account"
  has_one :profile
  scope :active, ->(since) { where(active: true) }
  enum status: { active: 0, archived: 1 }
  enum :role, { admin: 0, member: 1 }, prefix: true
  enum mode: { fast: 0, slow: 1 }
  self.table_name = "users"
end
